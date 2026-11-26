import { execFileSync } from 'node:child_process';
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const directory = process.argv[2];
const fixtureVersion = '0.2.5';
const version = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).version;
execFileSync('git', ['worktree', 'add', '--detach', directory, process.env.GITHUB_SHA], { stdio: 'inherit' });
writeFileSync(join(directory, 'upgrade-version.json'), JSON.stringify({ version: fixtureVersion }));
appendFileSync(join(directory, 'src/main.tsx'), `

import { appUpdateStatus, checkAppUpdate, installAppUpdate } from './bridge';

async function runUpgradeFixture() {
  const status = await appUpdateStatus();
  if (!status.supported || status.version !== ${JSON.stringify(fixtureVersion)}) {
    throw new Error('Unexpected upgrade fixture status: ' + JSON.stringify(status));
  }
  const update = await checkAppUpdate();
  if (update?.version !== ${JSON.stringify(version)}) {
    throw new Error('Unexpected update available to fixture: ' + JSON.stringify(update));
  }
  await installAppUpdate(update.version, () => {});
}

void runUpgradeFixture().catch(error => console.error('Upgrade fixture:', error));
`);
