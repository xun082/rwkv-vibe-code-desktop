import { readFileSync } from 'node:fs';
import { setTimeout } from 'node:timers/promises';

const { GITHUB_REPOSITORY, VERSION } = process.env;
const { endpoints: [endpoint] } = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).plugins.updater;
const prefix = `https://github.com/${GITHUB_REPOSITORY}/releases/download/v${VERSION}/RWKV-Vibe-Code_${VERSION}`;
const assets = {
  'windows-x86_64-nsis': `${prefix}_x64-setup.exe`,
  'linux-x86_64-deb': `${prefix}_amd64.deb`,
  'linux-x86_64-appimage': `${prefix}_amd64.AppImage`,
};

for (let attempt = 1; attempt <= 12; attempt++) {
  try {
    const response = await fetch(endpoint, { signal: AbortSignal.timeout(10_000) });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const manifest = await response.json();
    if (manifest.version !== VERSION) throw new Error(`Version ${manifest.version} is still being served`);
    for (const [format, url] of Object.entries(assets)) {
      const entry = manifest.platforms?.[format];
      if (entry?.url !== url || typeof entry.signature !== 'string' || entry.signature.length === 0) {
        throw new Error(`Invalid ${format} entry in the published feed`);
      }
    }
    console.log(`Published update feed lists signed packages for ${VERSION}.`);
    break;
  } catch (error) {
    if (attempt === 12) throw error;
    console.log(`Waiting for the published update feed (${error.message}).`);
    await setTimeout(5000);
  }
}
