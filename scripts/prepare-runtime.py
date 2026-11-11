#!/usr/bin/env python3
"""Freeze the tested macOS arm64 runtime; model weights are installed on first launch."""
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
RES = ROOT / 'src-tauri/resources'
RUNTIME = RES / 'runtime'



def run(*args):
    return subprocess.check_output(args, text=True).strip()


def dependencies(path):
    return [line.strip().split(' (compatibility')[0] for line in run('otool', '-L', str(path)).splitlines()[1:]]


def main():
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        raise SystemExit('This release packages the tested macOS arm64 runtime only.')
    server = Path(os.environ.get('LLAMA_SERVER_PATH') or shutil.which('llama-server') or '')
    if not server.is_file():
        raise SystemExit('Install runtime first: brew install llama.cpp')
    version_result = subprocess.run([str(server), '--version'], capture_output=True, text=True, check=True)
    version = version_result.stdout + version_result.stderr
    if 'cd959bae9' not in version:
        raise SystemExit('Runtime differs from tested cd959bae9. Revalidate before changing the pinned runtime.')
    RUNTIME.mkdir(parents=True, exist_ok=True)
    search = [server.resolve().parent.parent / 'lib', Path('/opt/homebrew/opt/ggml/lib'), Path('/opt/homebrew/opt/openssl@4/lib'), Path('/opt/homebrew/opt/libomp/lib')]
    copied = {}

    def resolve(dep, parent):
        if dep.startswith('/opt/homebrew/'):
            return Path(dep)
        if dep.startswith('@loader_path/'):
            return parent / dep.removeprefix('@loader_path/')
        if dep.startswith('@rpath/'):
            name = dep.removeprefix('@rpath/')
            for directory in [parent, *search]:
                if (directory / name).is_file():
                    return directory / name
        raise RuntimeError(f'Unresolved dependency: {dep}')

    def copy_binary(source, name=None):
        name = name or source.name
        if name in copied:
            return
        target = RUNTIME / name
        copied[name] = str(source.resolve())
        shutil.copy2(source, target)
        target.chmod(0o755)
        if name == 'libggml.0.dylib':
            # This pinned Homebrew build has GGML_BACKEND_DIR compiled into a C string.
            # Replace it in-place (without moving Mach-O sections) with an empty directory
            # string so GGML falls back to the executable directory. The caller also uses
            # runtime/ as cwd. This prevents silently loading the developer's GGML plugins.
            backend_dir = str(Path(run('brew', '--prefix', 'ggml')).resolve() / 'libexec').encode()
            binary = target.read_bytes()
            if binary.count(backend_dir + b'\0') != 1:
                raise RuntimeError('Pinned GGML backend-directory literal changed; rebuild or revalidate relocation.')
            target.write_bytes(binary.replace(backend_dir + b'\0', b'\0' * (len(backend_dir) + 1)))
        # All non-system dependencies are colocated and use loader-relative paths.
        for dep in dependencies(source):
            if dep.startswith(('/usr/lib/', '/System/Library/')):
                continue
            resolved = resolve(dep, source.resolve().parent)
            if resolved.resolve() == source.resolve():
                continue  # dylib install ID
            copy_binary(resolved, Path(dep).name)
            subprocess.run(['install_name_tool', '-change', dep, '@loader_path/' + Path(dep).name, str(target)], check=True, capture_output=True)
        if target.suffix in ('.dylib', '.so'):
            subprocess.run(['install_name_tool', '-id', '@loader_path/' + name, str(target)], check=True, capture_output=True)
        # Remove developer-machine search paths, including dlopen backend fallbacks.
        paths = re.findall(r'path (.+?) \(offset', run('otool', '-l', str(target)))
        for rpath in paths:
            if rpath.startswith('/opt/homebrew/'):
                subprocess.run(['install_name_tool', '-delete_rpath', rpath, str(target)], check=True, capture_output=True)
        subprocess.run(['codesign', '--force', '--sign', '-', str(target)], check=True, capture_output=True)

    copy_binary(server, 'llama-server')
    # GGML loads these dynamically, so otool's transitive dependencies alone are insufficient.
    for backend in Path('/opt/homebrew/opt/ggml/libexec').glob('*.so'):
        copy_binary(backend)
    for path in RUNTIME.iterdir():
        if path.name not in copied and path.is_file():
            path.unlink()
    for name in copied:
        for dep in dependencies(RUNTIME / name):
            if dep.startswith('/opt/homebrew/') or dep.startswith('@rpath/'):
                raise RuntimeError(f'Unrelocated dependency in {name}: {dep}')
    licenses = RUNTIME / 'licenses'
    licenses.mkdir(exist_ok=True)
    for formula in ['llama.cpp', 'ggml', 'openssl@4', 'libomp']:
        prefix = Path(run('brew', '--prefix', formula))
        for candidate in ['LICENSE', 'LICENSE.txt', 'LICENSE.TXT']:
            if (prefix / candidate).exists():
                shutil.copy2(prefix / candidate, licenses / f'{formula.replace("@", "-")}.txt')
                break
    model_spec = json.loads((ROOT / 'src-tauri/model-manifest.json').read_text())
    manifest = {'platform': 'macOS arm64', 'model_bundled': False, 'model_install': model_spec, 'runtime_version': version, 'binaries': copied}
    (RES / 'bundle-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    # Launch from an unrelated directory with an explicit backend path, independent of Homebrew libraries.
    env = dict(os.environ)
    env.pop('GGML_BACKEND_PATH', None)
    check = subprocess.run([str(RUNTIME / 'llama-server'), '--version'], cwd='/tmp', env=env, capture_output=True, text=True, check=True)
    print(check.stdout or check.stderr)
    print(f'Packaged {len(copied)} runtime binaries; model weights are excluded and installed on demand.')

if __name__ == '__main__':
    main()
