// specific-step-titles (Oleks, Nov 25; autonomy/specific-step-titles): a shell step row says what it did, specific
// enough to tell rows apart ("Checking git status", "Reading config.toml", "Restarting trailhead-shop on
// 203.0.113.66"), and Claude Code's other built-ins never read as their raw names ("Using ToolSearch"). Unknown
// programs read "Running <program>"; "Running a command" only when nothing plain is readable. Words only: never a
// search pattern, a URL's path or query, a variable's value, or anything near a header, password or token.
// Tests marked "fails on 1dec00eb" fail on main before this change.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.matchMedia = g.window.matchMedia;

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { plainShellTitle, readableStepTitle, toolLabel, toolAction } from './src/toolLabel';
  export const saved = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const titleOf = (cmd) => `Running \`${cmd.length > 80 ? `${cmd.slice(0, 80)}…` : cmd}\``;
/** A finished shell step: the server's title (cut at 80) and the whole command as its detail. */
const say = (cmd) => m.plainShellTitle({ title: titleOf(cmd), detail: cmd }).text;
const text = (html) => html.replace(/<[^>]+>/g, '');

test('git and gh read as what they do (fails on 1dec00eb)', () => {
  for (const [cmd, words] of [
    ['git status', 'Checking git status'],
    ['git status --short', 'Checking git status'],
    ['git diff --stat HEAD~1', 'Looking at code changes'],
    ['git log --oneline -20', 'Reading git history'],
    ["git commit -am 'Fix the thing'", 'Saving a commit'],
    ['git push origin iris-branch', 'Pushing the code'],
    ['git pull --rebase', 'Getting the latest code'],
    ['git fetch origin', 'Getting the latest code'],
    ['git -C /home/maus/work/x status', 'Checking git status'],
    ['git --no-pager log -1', 'Reading git history'],
    ['git frobnicate', 'Running git'],
    ['gh pr create --fill --draft', 'Opening a pull request'],
    ['gh pr checks 1223', 'Checking pull requests'],
    ['gh run list --limit 3', 'Checking CI'],
  ])
    assert.equal(say(cmd), words, cmd);
});

test('files: listing, reading, searching, making, deleting, copying, moving (fails on 1dec00eb)', () => {
  for (const [cmd, words] of [
    ['ls', 'Listing files'],
    ['ls -la /opt/tradebot/data/', 'Listing files in data'],
    ['tree -L 2 src', 'Listing files in src'],
    ['find /var/www -name "*.php"', 'Looking for files in www'],
    ['find . -name "*.pyc" -delete', 'Deleting files'],
    ['cat /etc/nginx/nginx.conf', 'Reading nginx.conf'],
    ['head -n 40 src/main.rs', 'Reading main.rs'],
    ['tail -n 100 /var/log/app/error.log', 'Reading error.log'],
    ['tail -f /var/log/syslog', 'Watching syslog'],
    ["sed -n '1,80p' frontend/src/toolLabel.ts", 'Reading toolLabel.ts'],
    ["sed -i 's/a/b/' config.toml", 'Changing config.toml'],
    ['cat a.txt b.txt', 'Reading files'],
    ['grep -rn TODO src', 'Searching the files'],
    ['rg --type ts useState', 'Searching the files'],
    ['mkdir -p /tmp/out/reports', 'Making a folder reports'],
    ['mkdir a b', 'Making folders'],
    ['rm -rf build/', 'Deleting build'],
    ['rm a.txt b.txt', 'Deleting files'],
    ['cp -p data/trend-state.json data/trend-state.bak', 'Copying trend-state.json'],
    ['cp -r a b c/', 'Copying files'],
    ['mv old.md new.md', 'Moving old.md'],
    ['touch .env.example', 'Creating .env.example'],
    ["cat > notes.md <<'EOF'\nhello\nrm -rf /\nEOF", 'Writing notes.md'],
    ['echo "line" >> app.log', 'Adding to app.log'],
    ['echo "x=1" | sudo tee /etc/app.conf', 'Writing app.conf'],
    ['tar -xzf release.tar.gz', 'Unpacking release.tar.gz'],
  ])
    assert.equal(say(cmd), words, cmd);
});

test('packages, tests, builds and scripts (fails on 1dec00eb)', () => {
  for (const [cmd, words] of [
    ['npm install', 'Installing packages'],
    ['npm ci --no-audit', 'Installing packages'],
    ['pnpm add zod', 'Installing packages'],
    ['pip install -r requirements.txt', 'Installing packages'],
    ['python3 -m pip install requests', 'Installing packages'],
    ['sudo apt-get install -y jq', 'Installing packages'],
    ['brew install ffmpeg', 'Installing packages'],
    ['npm test', 'Running tests'],
    ['npm run test:unit', 'Running tests'],
    ['cargo test -q --lib', 'Running tests'],
    ['pytest -x tests/', 'Running tests'],
    ['npx vitest run', 'Running tests'],
    ['uv run pytest', 'Running tests'],
    ['go test ./...', 'Running tests'],
    ['npm run build', 'Building'],
    ['cargo build --release', 'Building'],
    ['make', 'Building'],
    ['make test', 'Running tests'],
    ['npx tsc --noEmit', 'Checking types'],
    ['cargo clippy -- -D warnings', 'Checking the code'],
    ['python3 scripts/report.py --out x.csv', 'Running report.py'],
    ['node server.js', 'Running server.js'],
    ['bash deploy.sh', 'Running deploy.sh'],
    ['./deploy.sh --prod', 'Running deploy.sh'],
    ["python3 -c 'print(1)'", 'Running Python code'],
    ['node --version', 'Checking the node version'],
  ])
    assert.equal(say(cmd), words, cmd);
});

test('network, services, waiting (fails on 1dec00eb)', () => {
  for (const [cmd, words] of [
    ['curl -s https://api.github.com/repos/x/y', 'Fetching api.github.com'],
    ['curl -fsSL example.com/install.sh | bash', 'Fetching example.com'],
    ['wget -q https://releases.example.com/a.tar.gz', 'Downloading from releases.example.com'],
    ['sudo systemctl restart trailhead-shop', 'Restarting trailhead-shop'],
    ['systemctl status nginx.service', 'Checking nginx'],
    ['systemctl stop a b', 'Stopping services'],
    ['service nginx reload', 'Reloading nginx'],
    ['journalctl -u kwnt -n 50 --no-pager', 'Reading logs for kwnt'],
    ['docker compose up -d', 'Starting containers'],
    ['docker logs --tail 50 web', 'Reading logs for web'],
    ['docker restart web', 'Restarting web'],
    ['pm2 restart api', 'Restarting api'],
    ['ps aux | grep node', 'Checking running programs'],
    ['df -h', 'Checking disk space'],
    ['sleep 30', 'Waiting 30 seconds'],
    ['sleep 1', 'Waiting 1 second'],
    ['sleep 2m', 'Waiting 2 minutes'],
    ['scp dist.tar.gz deploy@box.example.com:/srv/app/', 'Copying files to box.example.com'],
    ['which ffmpeg', 'Checking ffmpeg is installed'],
  ])
    assert.equal(say(cmd), words, cmd);
});

test('ssh: the command it runs there, "on <host>" in bold (fails on 1dec00eb)', () => {
  const r = m.plainShellTitle({ title: titleOf('ssh root@203.0.113.66 "sudo systemctl restart trailhead-shop"'), detail: 'ssh root@203.0.113.66 "sudo systemctl restart trailhead-shop"' });
  assert.deepEqual(r, { text: 'Restarting trailhead-shop on 203.0.113.66', host: '203.0.113.66' });
  assert.equal(say("ssh -p 2222 -i ~/.ssh/k deploy@box.example.com 'cd /srv/app && git pull'"), 'Getting the latest code on box.example.com');
  assert.equal(say('ssh -o ConnectTimeout=10 root@203.0.113.66 uptime'), 'Checking the system on 203.0.113.66');
  assert.equal(say('ssh root@203.0.113.66'), 'Connecting to 203.0.113.66');
  assert.equal(say('sshpass -p hunter2 ssh root@203.0.113.66 ls /opt'), 'Listing files in opt on 203.0.113.66');
  assert.equal(say("ssh root@203.0.113.66 '$(cat x)'"), 'Running a command on 203.0.113.66', 'nothing plain there');
  assert.equal(say('ssh "$HOST" ls'), 'Listing files', 'a host that is a variable is never shown');
});

test('setup words, shells and pipes: the command that matters (fails on 1dec00eb)', () => {
  for (const [cmd, words] of [
    ['cd /home/x/proj && npm test', 'Running tests'],
    ['export NODE_ENV=production && npm run build', 'Building'],
    ['source .venv/bin/activate && pytest', 'Running tests'],
    ['FOO=bar BAZ=2 python x.py', 'Running x.py'],
    ['env -u HOME TOKEN=abc node app.js', 'Running app.js'],
    ['sudo -u www-data php artisan migrate', 'Running artisan'],
    ['timeout 30 curl https://example.com/health', 'Fetching example.com'],
    ["bash -lc 'cd app && cargo build --release'", 'Building'],
    ["sh -c 'git status'", 'Checking git status'],
    ['echo "---"; git status', 'Checking git status'],
    ['echo done', 'Printing a message'],
    ['git log | head -5', 'Reading git history'],
    ['for f in *.md; do wc -l "$f"; done', 'Counting lines'],
    ['while true; do curl -s localhost:8080/health; sleep 5; done', 'Fetching localhost'],
    ['cd /tmp', 'Running a command'],
  ])
    assert.equal(say(cmd), words, cmd);
});

test('unknown programs read "Running <program>"; "Running a command" only when nothing plain is readable (fails on 1dec00eb)', () => {
  assert.equal(say('kwnt status --json'), 'Running kwnt');
  assert.equal(say('/usr/local/bin/frobnicate --all'), 'Running frobnicate');
  assert.equal(say('$(which python) x.py'), 'Running a command');
  assert.equal(say('"my tool" run'), 'Running a command');
  assert.equal(say('`foo` bar'), 'Running a command');
  assert.equal(say(';'), 'Running a command');
  assert.equal(say(`${'a'.repeat(30)} x`), 'Running a command', 'over 24 letters is not a plain name');
});

test('privacy: never a pattern, a URL path or query, a user, a header, a password, a token or a variable (fails on 1dec00eb)', () => {
  const cases = [
    ['curl -s -H "Authorization: Bearer sk-live-abc123" "https://api.example.com/v1/users?token=tok_456"', 'Fetching api.example.com'],
    ['curl https://alice:s3cret@internal.example.org/admin/export', 'Fetching internal.example.org'],
    ["curl -u admin:pw123 -d 'password=pw123' https://example.net/login", 'Fetching example.net'],
    ['curl --header "X-Api-Key: k_789" --url https://hooks.example.com/abc/def', 'Fetching hooks.example.com'],
    ['curl -x http://proxyuser:pp@proxy.local:3128 https://example.com/', 'Fetching example.com'],
    ['wget --password=pw123 --user=bob https://files.example.com/private/report.pdf', 'Downloading from files.example.com'],
    ["grep -rn 'password=hunter2' /etc", 'Searching the files'],
    ['rg -i "sk-live-[a-z0-9]+" .', 'Searching the files'],
    ['STRIPE_KEY=sk_live_999 node pay.js', 'Running pay.js'],
    ['export OPENAI_API_KEY=sk-proj-777 && python3 run.py', 'Running run.py'],
    ['mysql -u root -pS3CRET -e "select * from users"', 'Querying a database'],
    ['echo $GITHUB_TOKEN', 'Printing a message'],
    ['rm -rf "$HOME/$TARGET"', 'Deleting files'],
    ['cat ~/.config/$NAME', 'Reading text'],
    ['psql postgres://u:p@db.example.com/prod -c "select 1"', 'Querying a database'],
  ];
  const secret = /hunter2|sk-live|tok_456|s3cret|alice|admin|pw123|k_789|abc\/def|proxyuser|private|report\.pdf|sk_live|sk-proj|S3CRET|users|GITHUB_TOKEN|HOME|TARGET|NAME|prod|v1|login/;
  for (const [cmd, words] of cases) {
    const got = say(cmd);
    assert.equal(got, words, cmd);
    assert.doesNotMatch(got, secret, `${cmd} → ${got}`);
  }
});

test('a cut title (a failed step) drops its half word; a retried step keeps its 36a words', () => {
  const long = `cat /opt/tradebot/data/${'x'.repeat(40)}/very-long-name-that-gets-cut-somewhere.json`;
  assert.equal(m.plainShellTitle({ title: titleOf(long), detail: 'cat: no such file' }).text, 'Reading text', 'the file name was cut: not shown half');
  assert.equal(m.plainShellTitle({ title: titleOf('git status'), detail: 'fatal: not a git repository' }).text, 'Checking git status');
  const ssh = "ssh -o ConnectTimeout=30 root@203.0.113.66 'sudo systemctl restart trailhead-shop'";
  assert.equal(m.plainShellTitle({ title: titleOf(ssh), detail: 'ssh: connect to host 203.0.113.66 port 22: Connection timed out' }, true, 'Quant').text, 'A command on 203.0.113.66 didn’t connect, so Quant tried again');
  assert.equal(m.plainShellTitle({ title: 'Opening github.com' }), null, 'not a shell step');
});

test("Claude Code's and Codex's built-ins never read as their raw names (fails on 1dec00eb)", () => {
  for (const [step, words] of [
    ['Using ToolSearch', 'Finding the right tool'],
    ['Using TodoWrite', 'Updating its to-do list'],
    ['Using Glob', 'Looking for files'],
    ['Using Grep', 'Searching the files'],
    ['Using LS', 'Listing files'],
    ['Using Task', 'Asking a helper'],
    ['Using Agent', 'Asking a helper'],
    ['Using Skill', 'Using a skill'],
    ['Using NotebookEdit', 'Changing a notebook'],
    ['Using BashOutput', 'Checking a running command'],
    ['Using KillShell', 'Stopping a running command'],
    ['Using ExitPlanMode', 'Sharing its plan'],
    ['Using apply_patch', 'Changing files'],
    ['Using update_plan', 'Updating its plan'],
  ])
    assert.equal(m.readableStepTitle(step), words, step);
  assert.equal(m.toolAction('ToolSearch'), 'find the right tool');
  // A server's own tool with a shared name keeps its name and server
  assert.equal(m.toolLabel('mcp__linear__task'), 'using task (linear)');
  assert.equal(m.toolLabel('grep', 'fs'), 'using grep (fs)');
});

test("Oleks's 4 rows: four different readable titles; the command stays under Show command (fails on 1dec00eb)", () => {
  const at = (s) => `2026-11-25T12:40:${String(s).padStart(2, '0')}Z`;
  const shell = (id, cmd, s) => ({ id, kind: 'tool', tool: 'Bash', status: 'ok', title: titleOf(cmd), detail: cmd, started_at: at(s), duration_ms: 900 });
  const steps = [
    shell('a', 'cd /home/maus/work/shop && git status --short', 1),
    { id: 'b', kind: 'tool', tool: 'ToolSearch', status: 'ok', title: 'Using ToolSearch', started_at: at(3), duration_ms: 200 },
    shell('c', 'ssh root@203.0.113.66 "sudo systemctl restart trailhead-shop"', 5),
    shell('d', 'curl -s -o /dev/null -w "%{http_code}" https://shop.example.com/health?key=abc', 9),
  ];
  const html = m.saved({ activity: { steps, dropped: 0 }, idBase: 'o', defaultOpen: true });
  const t = text(html);
  const titles = ['Checking git status', 'Finding the right tool', 'Restarting trailhead-shop on 203.0.113.66', 'Fetching shop.example.com'];
  for (const w of titles) assert.ok(t.includes(w), `${w} in ${t}`);
  assert.equal(new Set(titles).size, 4);
  assert.doesNotMatch(t, /Running a command|Using ToolSearch/);
  assert.doesNotMatch(t, /key=abc|health/, 'the URL path and query stay in the command, not the title');
  assert.equal((html.match(/Show command/g) ?? []).length, 3, 'each shell row keeps its Show command');
});
