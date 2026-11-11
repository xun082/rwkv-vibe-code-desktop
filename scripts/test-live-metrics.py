#!/usr/bin/env python3
"""One inference POST, 1–4 complete prompts, indexed live HTML + TPS and memory."""
import argparse
import json
import os
import re
import socket
import subprocess
import threading
import time
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'local-model-results'/'four-lane-batch'

class VisibleText(HTMLParser):
    def __init__(self):
        super().__init__(); self.body=False; self.blocked=0; self.heading=0; self.first_text=None; self.first_heading=None
    def handle_starttag(self, tag, attrs):
        if tag=='body': self.body=True
        if tag in ('script','style','noscript','template'): self.blocked+=1
        if tag=='h1': self.heading+=1
    def handle_endtag(self, tag):
        if tag in ('script','style','noscript','template'): self.blocked=max(0,self.blocked-1)
        if tag=='h1': self.heading=max(0,self.heading-1)
    def handle_data(self, data):
        if self.body and not self.blocked and re.search(r'[A-Za-z\u4e00-\u9fff]',data):
            self.first_text=self.first_text or data.strip()
            if self.heading: self.first_heading=self.first_heading or data.strip()

def main():
    global OUT
    cli=argparse.ArgumentParser()
    cli.add_argument('--concurrency',type=int,choices=(1,2,3,4),default=4)
    cli.add_argument('--max-tokens',type=int,default=12000)
    cli.add_argument('--transport-only',action='store_true')
    cli.add_argument('--runtime-dir',type=Path,default=ROOT/'src-tauri/resources/runtime')
    cli.add_argument('--strict-content',action='store_true',help='also require requested photo assets in every model response')
    args=cli.parse_args()
    OUT=ROOT/'local-model-results'/(f'batch-{args.concurrency}-lane'+('-transport'if args.transport_only else ''))
    OUT.mkdir(parents=True, exist_ok=True)
    for artifact in ('summary.json','validation.json'): (OUT/artifact).unlink(missing_ok=True)
    source=(ROOT/'src-tauri/src/prompt.rs').read_text(encoding='utf-8')
    def constant(name): return re.search(r'pub const '+name+r': &str = r#"(.*?)"#;',source,re.S).group(1)
    prefix=constant('WEBPAGE_PREFIX');instruction=constant('STREAMING_INSTRUCTION');prefill=constant('HTML_PREFILL')
    examples = json.loads(subprocess.check_output([
        'node', '--input-type=module', '-e',
        "import { EXAMPLE_PROMPTS } from './src/lib/examples.ts'; console.log(JSON.stringify(EXAMPLE_PROMPTS));",
    ], cwd=ROOT, encoding='utf-8'))
    prompts_by_key = {example['key']: example['prompt'] for example in examples}
    cases=['创建一个咖啡店官网，店名「慢半拍」，暖白和咖啡棕配色，介绍三款咖啡和营业时间。']+[prompts_by_key[key] for key in ('hotel','museum','saas')]
    cases=cases[:args.concurrency]
    model_name=json.loads((ROOT/'src-tauri/model-manifest.json').read_text(encoding='utf-8'))['filename']
    windows = os.name == 'nt'
    app_data = Path(os.environ['APPDATA']) if windows else Path.home()/'Library/Application Support'
    model_candidates=[Path(os.environ['RWKV_MODEL_PATH'])] if os.environ.get('RWKV_MODEL_PATH') else [app_data/'studio.moment.localpage/models'/model_name,Path.home()/'Models/rwkv'/model_name,ROOT/'src-tauri/resources/models'/model_name]
    model=next((candidate for candidate in model_candidates if candidate.is_file()),None)
    if model is None: raise RuntimeError('Open the desktop app to install the model first, or set RWKV_MODEL_PATH.')
    runtime=args.runtime_dir.resolve()
    server=runtime/('llama-server.exe' if windows else 'llama-server')
    model_arg=model.name if windows else str(model.resolve())
    working_dir=model.resolve().parent if windows else runtime
    with socket.socket() as sock: sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    env=dict(os.environ);env.pop('GGML_BACKEND_PATH',None)
    with (OUT/'runtime.log').open('w') as log:
        proc=subprocess.Popen([str(server),'-m',model_arg,'-ngl','0' if windows else '99','-np',str(args.concurrency),'-c',str(16384*args.concurrency),'--host','127.0.0.1','--port',str(port),'--jinja','--reasoning','off','--cache-ram','0','--slots'],cwd=working_dir,env=env,stdout=log,stderr=log)
        client=urllib.request.build_opener(urllib.request.ProxyHandler({}));base=f'http://127.0.0.1:{port}';audit=[];finished=threading.Event();monitor=None
        def post(path,payload):
            audit.append({'method':'POST','path':path,'promptCount':len(payload['prompt']) if isinstance(payload.get('prompt'),list) else None})
            return client.open(urllib.request.Request(base+path,data=json.dumps(payload).encode(),headers={'Content-Type':'application/json'}),timeout=300)
        try:
            for _ in range(160):
                if proc.poll() is not None: raise RuntimeError('server exited')
                try:
                    with client.open(base+'/health',timeout=1) as response:
                        if json.load(response).get('status')=='ok': break
                except Exception: time.sleep(.25)
            else: raise RuntimeError('server health timeout')
            loaded_images=[]
            if not windows:
                listing=subprocess.check_output(['lsof','-p',str(proc.pid),'-Fn'],text=True)
                loaded_images=sorted({line[1:]for line in listing.splitlines()if line.startswith('n')and line.endswith(('.so','.dylib'))})
                assert not any('/opt/homebrew/' in path for path in loaded_images), 'runtime loaded a development-machine dependency'
            marker='__LOCALPAGE_BATCH_REQUEST__'
            with post('/apply-template',dict(messages=[{'role':'system','content':instruction},{'role':'user','content':marker}],add_generation_prompt=True,chat_template_kwargs={'enable_thinking':False})) as response: template=json.load(response)['prompt']+prefill
            (OUT/'template.txt').write_text(template)
            assert template.count(marker)==1 and template.endswith(prefill)
            prompts=[template.replace(marker,prefix+' '+case)for case in cases]
            assert len(set(prompts))==args.concurrency
            payload=dict(prompt=prompts,stream=True,n_predict=args.max_tokens,stop=['</html>'],temperature=1.0,top_p=.5,top_k=60,min_p=0,repeat_penalty=1,frequency_penalty=.1,presence_penalty=1,timings_per_token=True,return_progress=True,dry_multiplier=.8,dry_allowed_length=48,dry_penalty_last_n=2048,dry_sequence_breakers=['<','>'],seed=2026)
            (OUT/'request.json').write_text(json.dumps(payload,ensure_ascii=False,indent=2)+'\n')
            start=time.monotonic();slots=[];states=[];interleaved=[];seen_indices=set();rss_samples=[]
            for i,case in enumerate(cases):
                parser=VisibleText();parser.feed(prefill)
                states.append(dict(index=i,prompt=case,text=prefill,parser=parser,samples=[],first_body=None,first_heading=None,done=False))
            def observe_slots():
                while not finished.is_set():
                    try:
                        with client.open(base+'/slots',timeout=2) as response: data=json.load(response)
                        slots.append({'at_s':time.monotonic()-start,'busy':sum(bool(slot['is_processing'])for slot in data)})
                    except Exception: pass
                    if not windows:
                        try:
                            rss=int(subprocess.check_output(['ps','-o','rss=','-p',str(proc.pid)],text=True).strip());rss_samples.append(rss)
                        except Exception: pass
                    finished.wait(.5)
            monitor=threading.Thread(target=observe_slots);monitor.start();last_saved=0
            with post('/completion',payload) as response:
                for line in response:
                    if not line.startswith(b'data:'): continue
                    event=json.loads(line[5:])
                    if event.get('error'): raise RuntimeError(event['error'])
                    seen_indices.add(event['index'])
                    state=states[event['index']];assert not state['done'];now=time.monotonic()-start
                    delta=event.get('content') or '';state['text']+=delta;state['parser'].feed(delta)
                    timing=event.get('timings')
                    if timing: state['samples'].append(dict(at_s=now,tokens=timing['predicted_n'],tps=timing['predicted_per_second']))
                    if delta and len(interleaved)<40: interleaved.append(event['index'])
                    for key,attribute in [('first_body','first_text'),('first_heading','first_heading')]:
                        visible=getattr(state['parser'],attribute)
                        if not state[key] and visible:
                            state[key]=dict(at_s=now,tokens=timing['predicted_n'] if timing else None,text=visible)
                            print(key,state['index'],state[key],flush=True)
                    if event.get('stop'):
                        state['done']=True;state['final_event']={k:event[k]for k in ('tokens_predicted','tokens_evaluated','stop_type','stopping_word','truncated','timings')}
                        if event.get('stopping_word')=='</html>':state['text']+='</html>'
                        state['finished_s']=now
                    if time.monotonic()-last_saved>.5 or event.get('stop'):
                        for item in states: (OUT/f"page-{item['index']+1}.html").write_text(item['text'])
                        last_saved=time.monotonic()
                    if all(item['done']for item in states): break
            finished.set();monitor.join();report=[]
            for state in states:
                text=state['text']
                state['contentChecks']=dict(sectionCount=text.count('<section'),hasNav='<nav' in text,hasFooter='<footer' in text,hasUnsplash='images.unsplash.com' in text,closedHtml='</html>' in text)
                report.append({k:v for k,v in state.items()if k not in ('parser','text')})
            result=dict(loadedRuntimeImages=loaded_images,inferenceRequests=sum(item['path']=='/completion'for item in audit),promptCount=len(prompts),differentPrompts=True,peakRssMiB=max(rss_samples,default=0)/1024,seenIndices=sorted(seen_indices),transportOnly=args.transport_only,peakBusySlots=max(item['busy']for item in slots),firstStreamIndices=interleaved,httpRequests=audit,jobs=report,slotSamples=slots)
            assert result['inferenceRequests']==1 and result['peakBusySlots']==args.concurrency and seen_indices==set(range(args.concurrency))
            (OUT/'validation.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
            summary={**result,'jobs':[{k:v for k,v in item.items()if k!='samples'}for item in report]};summary.pop('slotSamples')
            (OUT/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n')
            for state in states:
                samples=state['samples'];text=state['text']
                state['contentChecks']=dict(sectionCount=text.count('<section'),hasNav='<nav' in text,hasFooter='<footer' in text,hasUnsplash='images.unsplash.com' in text,closedHtml='</html>' in text)
                assert state['done']
                assert samples[-1]['tokens'] > 0 and len(text) > len(prefill)
                assert samples[-1]['tokens']==state['final_event']['tokens_predicted']
                assert all(a['tokens']<=b['tokens']for a,b in zip(samples,samples[1:]))
                if not args.transport_only:
                    assert '</html>' in text and state['final_event']['stop_type']=='word' and not state['final_event']['truncated']
                    assert state['first_body'] and state['first_heading']
                    assert text.count('<section')>=4 and '<nav' in text and '<footer' in text
                    if args.strict_content: assert 'images.unsplash.com' in text, 'model omitted requested photos'
                print('FINISHED',state['index'],state['finished_s'],samples[-1],flush=True)
            print(json.dumps(summary,ensure_ascii=False,indent=2),flush=True)
        finally:
            finished.set()
            if monitor: monitor.join(timeout=3)
            proc.terminate()
            try:proc.wait(timeout=10)
            except subprocess.TimeoutExpired:proc.kill();proc.wait()
if __name__=='__main__': main()
