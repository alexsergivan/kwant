// specific-step-titles (Nov 25): what a shell step did, in plain words specific enough to tell rows apart
// ("Checking git status", "Reading config.toml", "Restarting trailhead-shop"), from the command's own words.
// The exact command stays behind "Show command".
//
// Words only, and only safe ones: a program's name, a file's or folder's own name (never its path), a hostname
// (never a URL's path, query or user), a service name, a plain number of seconds. Never a search pattern, a
// variable's value, a header, or anything after a flag that names a password, token, key, header or user.
// Nothing is run, resolved or looked up.

/** One shell word: its text with quotes taken off, and whether it is dynamic ($, `, a glob), so never shown. */
interface Word {
  text: string;
  dynamic: boolean;
}

interface Redir {
  op: string;
  target: Word | null;
}

interface Segment {
  words: Word[];
  redirs: Redir[];
  /** The operator that ended this segment ("|", "&&", ";", …), or "" at the end. */
  next: string;
}

/** What a command did, in words, and the host when it ran over ssh (set in bold by the row). */
export interface ShellWords {
  text: string;
  host: string | null;
}

// ---- tokenizer ----

/**
 * The command cut into segments at `&&`, `||`, `;`, `|`, `&`, newlines and parentheses, each with its words and
 * redirects. Quotes are taken off; `$(…)`, backticks and `$VAR` mark a word dynamic. A heredoc's body is skipped.
 * Forgiving: an unclosed quote runs to the end.
 */
function segments(cmd: string): Segment[] {
  const out: Segment[] = [];
  let words: Word[] = [];
  let redirs: Redir[] = [];
  let cur = '';
  let dyn = false;
  let inWord = false;
  let wantTarget: string | null = null;
  const heredocs: string[] = [];

  const endWord = () => {
    if (!inWord) return;
    const w = { text: cur, dynamic: dyn };
    if (wantTarget !== null) {
      if (wantTarget === '<<' || wantTarget === '<<-') heredocs.push(cur);
      else redirs.push({ op: wantTarget, target: w });
      wantTarget = null;
    } else words.push(w);
    cur = '';
    dyn = false;
    inWord = false;
  };
  const endSeg = (op: string) => {
    endWord();
    if (wantTarget !== null) {
      redirs.push({ op: wantTarget, target: null });
      wantTarget = null;
    }
    if (words.length || redirs.length) out.push({ words, redirs, next: op });
    words = [];
    redirs = [];
  };

  let i = 0;
  const n = cmd.length;
  while (i < n) {
    const c = cmd[i];
    if (c === '\n') {
      endSeg(';');
      i++;
      // A heredoc's body runs to the line that is only its delimiter.
      while (heredocs.length && i < n) {
        const end = cmd.indexOf('\n', i);
        const line = cmd.slice(i, end < 0 ? n : end);
        i = end < 0 ? n : end + 1;
        if (line.trim() === heredocs[0]) heredocs.shift();
      }
      continue;
    }
    if (c === ' ' || c === '\t' || c === '\r') {
      endWord();
      i++;
      continue;
    }
    if (c === '\\') {
      inWord = true;
      if (i + 1 < n && cmd[i + 1] !== '\n') cur += cmd[i + 1];
      i += 2;
      continue;
    }
    if (c === "'") {
      inWord = true;
      const end = cmd.indexOf("'", i + 1);
      cur += cmd.slice(i + 1, end < 0 ? n : end);
      i = end < 0 ? n : end + 1;
      continue;
    }
    if (c === '"') {
      inWord = true;
      let j = i + 1;
      while (j < n && cmd[j] !== '"') {
        if (cmd[j] === '\\' && j + 1 < n) {
          cur += cmd[j + 1];
          j += 2;
          continue;
        }
        if (cmd[j] === '$' || cmd[j] === '`') dyn = true;
        cur += cmd[j];
        j++;
      }
      i = j + 1;
      continue;
    }
    if (c === '$' || c === '`') {
      inWord = true;
      dyn = true;
      // $( … ) and ` … ` stay inside this word, however deep.
      if (c === '$' && cmd[i + 1] === '(') {
        let depth = 0;
        let j = i + 1;
        for (; j < n; j++) {
          if (cmd[j] === '(') depth++;
          else if (cmd[j] === ')' && --depth === 0) break;
        }
        cur += cmd.slice(i, j + 1);
        i = j + 1;
        continue;
      }
      if (c === '`') {
        const end = cmd.indexOf('`', i + 1);
        cur += cmd.slice(i, end < 0 ? n : end + 1);
        i = end < 0 ? n : end + 1;
        continue;
      }
      cur += c;
      i++;
      continue;
    }
    if (c === '&' || c === '|' || c === ';') {
      // &> and &>> are redirects, not "and"
      if (c === '&' && cmd[i + 1] === '>') {
        endWord();
        const op = cmd[i + 2] === '>' ? '>>' : '>';
        i += op.length + 1;
        wantTarget = op;
        continue;
      }
      const two = cmd.slice(i, i + 2);
      const op = two === '&&' || two === '||' || two === ';;' ? two : c;
      endSeg(op);
      i += op.length;
      continue;
    }
    if (c === '(' || c === ')') {
      endSeg(c);
      i++;
      continue;
    }
    if (c === '>' || c === '<') {
      // "2>" and "1>>": the digits are the stream, not a word
      if (inWord && /^\d+$/.test(cur) && !dyn) {
        cur = '';
        inWord = false;
      } else endWord();
      let j = i + 1;
      while (j < n && (cmd[j] === '>' || cmd[j] === '<' || cmd[j] === '-')) j++;
      const op = cmd.slice(i, j);
      // 2>&1, >&2: to another stream, no file
      if (cmd[j] === '&') {
        j++;
        while (j < n && /[0-9-]/.test(cmd[j])) j++;
        i = j;
        continue;
      }
      i = j;
      wantTarget = op;
      continue;
    }
    if ((c === '*' || c === '?' || c === '[' || c === '{' || c === '~') && !(c === '{' && !inWord && /\s/.test(cmd[i + 1] ?? ' '))) dyn = true;
    inWord = true;
    cur += c;
    i++;
  }
  endSeg('');
  return out;
}

// ---- safe words ----

/** A file's or folder's own name, when it is plain: no path, no glob or variable, no "=" or ":". Else null. */
function nameOf(w: Word | undefined): string | null {
  if (!w || w.dynamic) return null;
  const t = w.text.replace(/\/+$/, '');
  const base = t.slice(t.lastIndexOf('/') + 1);
  if (base === '.' || base === '..' || base.startsWith('-')) return null;
  return /^[A-Za-z0-9._@+-]{1,60}$/.test(base) && /[A-Za-z0-9]/.test(base) ? base : null;
}

/** A program's name: the file name of a plain word, 1 to 24 letters, digits, ".", "_", "+" or "-" (server `shell_program`). */
function programOf(w: Word | undefined): string | null {
  if (!w || w.dynamic || !/^[A-Za-z0-9._+\-/~]+$/.test(w.text)) return null;
  const p = w.text.slice(w.text.lastIndexOf('/') + 1);
  return p.length >= 1 && p.length <= 24 && /^[A-Za-z0-9._+-]+$/.test(p) && /[A-Za-z0-9]/.test(p) ? p : null;
}

/** A plain hostname or IP from "host", "user@host:port" or "[v6]:port"; null when it isn't plain. */
function hostOf(dest: string): string | null {
  let h = dest.slice(dest.lastIndexOf('@') + 1);
  if (/^\[[0-9a-fA-F:.]+\](:\d+)?$/.test(h)) h = h.slice(1, h.indexOf(']'));
  else h = h.replace(/:\d*$/, '');
  if (/^[0-9a-fA-F:]+$/.test(h) && h.includes(':')) return h;
  return /^[A-Za-z0-9]([A-Za-z0-9-]{0,62})(\.[A-Za-z0-9-]{1,63})*$/.test(h) && h.length <= 253 ? h : null;
}

/** The hostname of a URL ("https://u:p@api.example.com:8443/x?y" → "api.example.com"); never its path, query or user. */
function urlHost(w: Word): string | null {
  const m = /^[A-Za-z][A-Za-z0-9+.-]*:\/\/([^/?#\s]*)/.exec(w.text);
  if (m) return w.dynamic && /[$`]/.test(m[1]) ? null : hostOf(m[1]);
  if (w.dynamic) return null;
  // curl and wget take "example.com/x" too: only when the first part has a dot
  const first = w.text.split(/[/?#]/)[0];
  return first.includes('.') || /^localhost(:\d+)?$/.test(first) ? hostOf(first) : null;
}

/** Flags whose value is never shown, whatever the program: anything naming a password, token, key, header or user. */
const SECRET_FLAG = /pass|token|secret|key|auth|header|cookie|user|cred|bearer|data|form/i;

/**
 * The words after a program that aren't flags, skipping the value of each flag in `takes` (and of any flag that
 * names a secret). "--" ends the flags. A flag written "--x=value" carries its own value.
 */
function positional(args: Word[], takes = ''): Word[] {
  const valued = new Set(takes.split(' ').filter(Boolean));
  const out: Word[] = [];
  let flags = true;
  for (let i = 0; i < args.length; i++) {
    const t = args[i].text;
    if (flags && t === '--') {
      flags = false;
      continue;
    }
    if (flags && t.length > 1 && t.startsWith('-')) {
      if (!t.includes('=') && (valued.has(t) || (t.startsWith('--') && SECRET_FLAG.test(t)))) i++;
      continue;
    }
    out.push(args[i]);
  }
  return out;
}

const hasFlag = (args: Word[], ...names: string[]) =>
  args.some((a) => names.includes(a.text) || names.some((f) => /^-[A-Za-z]$/.test(f) && /^-[A-Za-z]+$/.test(a.text) && a.text.includes(f[1])));

/** "Reading config.toml" for one plain name, else the plain words ("Reading files"). */
const named = (verb: string, w: Word[], one: string, many: string) => {
  if (w.length === 1) {
    const n = nameOf(w[0]);
    return n ? `${verb} ${n}` : one;
  }
  return w.length ? many : one;
};

const cap = (t: string) => t.charAt(0).toUpperCase() + t.slice(1);

// ---- programs ----

type Rule = (args: Word[], seg: Segment, all: Segment[], at: number, depth: number) => string | ShellWords | null;

const TESTS = /^(test|tests|t|spec|e2e|unit|check:test)([:_-].*)?$/;
const BUILDS = /^(build|compile|bundle|dist)([:_-].*)?$/;

/** An npm-style script name in words. */
function script(s: Word | undefined): string | null {
  if (!s) return null;
  const t = s.text;
  if (TESTS.test(t)) return 'Running tests';
  if (BUILDS.test(t)) return 'Building';
  if (/^(dev|start|serve|preview)([:_-].*)?$/.test(t)) return 'Starting the app';
  if (/^(lint|check|typecheck|tsc|types)([:_-].*)?$/.test(t)) return 'Checking the code';
  if (/^(format|fmt|prettier)([:_-].*)?$/.test(t)) return 'Formatting the code';
  const p = programOf(s);
  return p ? `Running ${p}` : null;
}

function pkg(args: Word[], seg: Segment, all: Segment[], at: number, depth: number, prog: string): string | ShellWords | null {
  const pos = positional(args, '--prefix -C --filter -w --workspace');
  const sub = pos[0]?.text;
  if (!sub) return prog === 'yarn' || prog === 'bun' ? 'Installing packages' : null;
  if (/^(install|i|ci|add|isntall)$/.test(sub)) return 'Installing packages';
  if (/^(uninstall|remove|rm|un)$/.test(sub)) return 'Removing packages';
  if (/^(update|upgrade|up)$/.test(sub)) return 'Updating packages';
  if (/^(test|t|tst)$/.test(sub)) return 'Running tests';
  if (sub === 'start') return 'Starting the app';
  if (sub === 'audit' || sub === 'outdated' || sub === 'ls' || sub === 'list') return 'Checking packages';
  if (sub === 'view' || sub === 'info' || sub === 'show') return 'Looking up a package';
  if (sub === 'publish') return 'Publishing a package';
  if (sub === 'run' || sub === 'run-script') return script(pos[1]) ?? `Running ${prog}`;
  if (sub === 'exec' || sub === 'dlx' || sub === 'x') return inner(args.slice(args.indexOf(pos[0]) + 1), seg, all, at, depth);
  if (prog === 'bun' && /\.(m?[jt]sx?)$/.test(sub)) return named('Running', [pos[0]], 'Running a script', 'Running a script');
  // yarn build, pnpm lint: a script by name
  if (prog !== 'npm') return script(pos[0]);
  return null;
}

/** Runs the rest of the words as their own command ("npx vitest" reads as "vitest"). */
function inner(rest: Word[], seg: Segment, all: Segment[], at: number, depth: number): string | ShellWords | null {
  if (!rest.length) return null;
  return analyse({ ...seg, words: rest }, all, at, depth + 1);
}

function interp(lang: string): Rule {
  return (args) => {
    for (let i = 0; i < args.length; i++) {
      const t = args[i].text;
      if (t === '-c' || t === '-e' || t === '--eval' || t === '-p' || t === '--print') return `Running ${lang} code`;
      if (t === '-m' && lang === 'Python') {
        const mod = args[i + 1]?.text ?? '';
        if (mod === 'pytest' || mod === 'unittest' || mod === 'tox') return 'Running tests';
        if (mod === 'pip') return pip(args.slice(i + 2));
        if (mod === 'http.server') return 'Starting a web server';
        if (mod === 'venv' || mod === 'virtualenv') return 'Making a Python environment';
        const p = programOf(args[i + 1]);
        return p ? `Running ${p}` : `Running ${lang} code`;
      }
      if (t.startsWith('-')) continue;
      return named('Running', [args[i]], `Running a ${lang} script`, `Running a ${lang} script`);
    }
    return `Starting ${lang}`;
  };
}

function pip(args: Word[]): string {
  const sub = positional(args)[0]?.text;
  if (sub === 'install') return 'Installing packages';
  if (sub === 'uninstall') return 'Removing packages';
  return 'Checking packages';
}

const sys = (verbs: Record<string, string>, fallback: string) => (args: Word[]) => {
  const pos = positional(args);
  const sub = pos[0]?.text ?? '';
  const v = Object.prototype.hasOwnProperty.call(verbs, sub) ? verbs[sub] : null;
  return v ?? fallback;
};

const SERVICE_VERB: Record<string, string> = {
  restart: 'Restarting',
  'try-restart': 'Restarting',
  'reload-or-restart': 'Restarting',
  start: 'Starting',
  stop: 'Stopping',
  reload: 'Reloading',
  status: 'Checking',
  'is-active': 'Checking',
  'is-failed': 'Checking',
  show: 'Checking',
  enable: 'Turning on',
  disable: 'Turning off',
  kill: 'Stopping',
};

/** "trailhead-shop.service" → "trailhead-shop". */
const unitName = (w: Word | undefined) => {
  const n = nameOf(w);
  return n ? n.replace(/\.service$/, '') : null;
};

function service(verbWord: string, units: Word[]): string | null {
  const verb = SERVICE_VERB[verbWord];
  if (!verb) return null;
  const u = units.length === 1 ? unitName(units[0]) : null;
  return u ? `${verb} ${u}` : units.length > 1 ? `${verb} services` : `${verb} a service`;
}

function fetchHost(args: Word[], takes: string): string | null {
  const valued = new Set(takes.split(' '));
  for (let i = 0; i < args.length; i++) {
    const t = args[i].text;
    if (t === '--url') return args[i + 1] ? urlHost(args[i + 1]) : null;
    if (t.startsWith('--url=')) return urlHost({ text: t.slice(6), dynamic: args[i].dynamic });
    if (t.length > 1 && t.startsWith('-')) {
      if (!t.includes('=') && (valued.has(t) || SECRET_FLAG.test(t) || /^-[A-Za-z]*[HdFuxeAbcoOTwmEKrCyYzU]$/.test(t))) i++;
      continue;
    }
    const h = urlHost(args[i]);
    if (h) return h;
  }
  return null;
}

const CURL_TAKES =
  '-H --header -d --data --data-raw --data-binary --data-urlencode --json -F --form -u --user -o --output -X --request -A -e --referer -b --cookie -c --cookie-jar -T --upload-file -w --write-out -m --max-time --connect-timeout -x --proxy --resolve -K --config -E --cert --key --cacert -r --range -C --retry --retry-delay -y -Y -z --oauth2-bearer --aws-sigv4 -U --proxy-user';
const WGET_TAKES = '-O -o -a -P -U --header --user --password --post-data --post-file -e -t -T -w --output-document --directory-prefix --load-cookies --save-cookies';

function git(args: Word[]): string {
  let i = 0;
  while (i < args.length && args[i].text.startsWith('-')) i += /^(-C|-c|--git-dir|--work-tree|--namespace)$/.test(args[i].text) ? 2 : 1;
  const sub = args[i]?.text ?? '';
  const GIT: Record<string, string> = {
    status: 'Checking git status',
    diff: 'Looking at code changes',
    log: 'Reading git history',
    shortlog: 'Reading git history',
    reflog: 'Reading git history',
    show: 'Looking at a commit',
    blame: 'Reading file history',
    commit: 'Saving a commit',
    push: 'Pushing the code',
    pull: 'Getting the latest code',
    fetch: 'Getting the latest code',
    clone: 'Copying a repository',
    add: 'Adding changes',
    checkout: 'Switching branches',
    switch: 'Switching branches',
    branch: 'Looking at branches',
    merge: 'Merging changes',
    rebase: 'Rebasing the branch',
    'cherry-pick': 'Copying a commit',
    stash: 'Setting changes aside',
    reset: 'Resetting changes',
    restore: 'Restoring files',
    revert: 'Undoing a commit',
    tag: 'Working with tags',
    remote: 'Checking remotes',
    worktree: 'Setting up a worktree',
    init: 'Starting a repository',
    config: 'Checking git settings',
    'rev-parse': 'Checking git',
    'ls-files': 'Listing files',
    grep: 'Searching the files',
    rm: 'Deleting files',
    mv: 'Moving files',
    apply: 'Changing files',
    am: 'Changing files',
    bisect: 'Looking for a change',
    submodule: 'Updating submodules',
    clean: 'Cleaning up files',
  };
  return Object.prototype.hasOwnProperty.call(GIT, sub) ? GIT[sub] : 'Running git';
}

function gh(args: Word[]): string {
  const [a, b] = positional(args, '-R --repo').map((w) => w.text);
  if (a === 'pr') {
    if (b === 'create') return 'Opening a pull request';
    if (b === 'merge') return 'Merging a pull request';
    if (b === 'review') return 'Reviewing a pull request';
    if (b === 'comment') return 'Commenting on a pull request';
    if (b === 'checkout') return 'Switching branches';
    return 'Checking pull requests';
  }
  if (a === 'issue') return b === 'create' ? 'Opening an issue' : b === 'comment' ? 'Commenting on an issue' : 'Looking at issues';
  if (a === 'run' || a === 'workflow') return 'Checking CI';
  if (a === 'release') return b === 'create' ? 'Making a release' : 'Looking at releases';
  if (a === 'repo') return b === 'clone' ? 'Copying a repository' : 'Looking at a repository';
  if (a === 'api') return 'Asking GitHub';
  if (a === 'auth') return 'Checking the GitHub sign-in';
  return 'Using GitHub';
}

function docker(args: Word[], compose: boolean): string {
  const pos = positional(args, '-f --file -p --project-name --context -H --host --env-file');
  let sub = pos[0]?.text ?? '';
  let rest = pos.slice(1);
  if (!compose && sub === 'compose') return docker(args.slice(args.findIndex((w) => w.text === 'compose') + 1), true);
  if (!compose && sub === 'container' && rest[0]) {
    sub = rest[0].text;
    rest = rest.slice(1);
  }
  const one = (verb: string, fb: string) => {
    const n = rest.length === 1 ? nameOf(rest[0]) : null;
    return n ? `${verb} ${n}` : fb;
  };
  if (compose) {
    if (sub === 'up' || sub === 'start') return 'Starting containers';
    if (sub === 'down' || sub === 'stop') return 'Stopping containers';
    if (sub === 'restart') return 'Restarting containers';
    if (sub === 'logs') return 'Reading container logs';
    if (sub === 'ps') return 'Checking containers';
    if (sub === 'build') return 'Building images';
    if (sub === 'pull') return 'Downloading images';
    if (sub === 'exec' || sub === 'run') return 'Running a command in a container';
    return 'Using Docker';
  }
  if (sub === 'ps' || sub === 'stats' || sub === 'inspect') return 'Checking containers';
  if (sub === 'images') return 'Listing images';
  if (sub === 'logs') {
    const n = positional(args.slice(args.findIndex((w) => w.text === 'logs') + 1), '--tail -n --since --until')[0];
    const name = nameOf(n);
    return name ? `Reading logs for ${name}` : 'Reading container logs';
  }
  if (sub === 'restart') return one('Restarting', 'Restarting containers');
  if (sub === 'start') return one('Starting', 'Starting containers');
  if (sub === 'stop' || sub === 'kill') return one('Stopping', 'Stopping containers');
  if (sub === 'rm' || sub === 'rmi') return 'Removing containers';
  if (sub === 'build' || sub === 'buildx') return 'Building an image';
  if (sub === 'run') return 'Starting a container';
  if (sub === 'pull') return 'Downloading an image';
  if (sub === 'push') return 'Uploading an image';
  if (sub === 'exec') return 'Running a command in a container';
  return 'Using Docker';
}

/** cat/echo/printf writing a file through a redirect or `| tee file`: "Writing notes.md", "Adding to app.log". */
function written(seg: Segment, all: Segment[], at: number): string | null {
  const r = seg.redirs.find((x) => x.op === '>' || x.op === '>>' || x.op === '>|');
  if (r) return r.target ? named(r.op === '>>' ? 'Adding to' : 'Writing', [r.target], 'Writing a file', 'Writing a file') : 'Writing a file';
  if (seg.next === '|') {
    const tee = all[at + 1];
    const w = tee ? stripPrefix(tee.words) : [];
    if (programOf(w[0]) === 'tee') {
      const files = positional(w.slice(1));
      return named(hasFlag(w.slice(1), '-a', '--append') ? 'Adding to' : 'Writing', files, 'Writing a file', 'Writing files');
    }
  }
  return null;
}

const files = (verb: string, one: string, many: string, takes = ''): Rule => (args) => named(verb, positional(args, takes), one, many);

const RULES: Record<string, Rule> = {
  git: (a) => git(a),
  gh: (a) => gh(a),
  ls: files('Listing files in', 'Listing files', 'Listing files', '-I --ignore -w --width -T --tabsize --color --format --sort --time-style'),
  tree: files('Listing files in', 'Listing files', 'Listing files', '-L -I -P -o --charset'),
  exa: files('Listing files in', 'Listing files', 'Listing files'),
  eza: files('Listing files in', 'Listing files', 'Listing files'),
  find: (a) => {
    if (a.some((w) => w.text === '-delete') || a.some((w, i) => (w.text === '-exec' || w.text === '-execdir') && /^(rm|rmdir)$/.test(a[i + 1]?.text ?? ''))) return 'Deleting files';
    const start: Word[] = [];
    for (const w of a) {
      if (/^[-(!]/.test(w.text)) break;
      start.push(w);
    }
    return named('Looking for files in', start, 'Looking for files', 'Looking for files');
  },
  fd: () => 'Looking for files',
  locate: () => 'Looking for files',
  du: () => 'Checking folder sizes',
  df: () => 'Checking disk space',
  free: () => 'Checking memory',
  cat: (a, seg, all, at) => {
    const w = written(seg, all, at);
    if (w && !positional(a).length) return w;
    return named('Reading', positional(a), 'Reading text', 'Reading files');
  },
  head: files('Reading', 'Reading text', 'Reading files', '-n -c --lines --bytes'),
  tail: (a) => {
    const f = positional(a, '-n -c --lines --bytes --pid -s --sleep-interval');
    return hasFlag(a, '-f', '-F', '--follow') ? named('Watching', f, 'Watching a file', 'Watching files') : named('Reading', f, 'Reading text', 'Reading files');
  },
  less: files('Reading', 'Reading text', 'Reading files'),
  more: files('Reading', 'Reading text', 'Reading files'),
  bat: files('Reading', 'Reading text', 'Reading files', '-l --language -r --line-range --style --theme'),
  nl: files('Reading', 'Reading text', 'Reading files'),
  wc: files('Counting lines in', 'Counting lines', 'Counting lines'),
  sed: (a) => {
    const scripted = a.some((w) => w.text === '-e' || w.text === '-f' || w.text.startsWith('--expression'));
    const pos = positional(a, '-e -f --expression --file -l');
    const f = scripted ? pos : pos.slice(1);
    const inPlace = a.some((w) => /^(-i|--in-place)/.test(w.text) || /^-[a-zA-Z]*i/.test(w.text));
    if (inPlace) return named('Changing', f, 'Changing a file', 'Changing files');
    if (hasFlag(a, '-n')) return named('Reading', f, 'Reading text', 'Reading files');
    return named('Editing text in', f, 'Editing text', 'Editing text');
  },
  awk: (a) => named('Reading', positional(a, '-F -v -f').slice(1), 'Processing text', 'Reading files'),
  jq: (a) => named('Reading', positional(a, '--arg --argjson --slurpfile --rawfile --args --indent -L').slice(1), 'Reading JSON', 'Reading JSON'),
  grep: () => 'Searching the files',
  egrep: () => 'Searching the files',
  fgrep: () => 'Searching the files',
  rg: () => 'Searching the files',
  ag: () => 'Searching the files',
  ack: () => 'Searching the files',
  echo: (_a, seg, all, at) => written(seg, all, at) ?? 'Printing a message',
  printf: (_a, seg, all, at) => written(seg, all, at) ?? 'Printing a message',
  tee: (a) => named(hasFlag(a, '-a', '--append') ? 'Adding to' : 'Writing', positional(a), 'Writing a file', 'Writing files'),
  mkdir: files('Making a folder', 'Making a folder', 'Making folders', '-m --mode'),
  rm: files('Deleting', 'Deleting files', 'Deleting files'),
  rmdir: files('Deleting', 'Deleting a folder', 'Deleting folders'),
  unlink: files('Deleting', 'Deleting a file', 'Deleting files'),
  trash: files('Deleting', 'Deleting files', 'Deleting files'),
  cp: (a) => {
    const p = positional(a, '-t --target-directory -S --suffix');
    return p.length === 2 ? named('Copying', [p[0]], 'Copying files', 'Copying files') : 'Copying files';
  },
  mv: (a) => {
    const p = positional(a, '-t --target-directory -S --suffix');
    return p.length === 2 ? named('Moving', [p[0]], 'Moving files', 'Moving files') : 'Moving files';
  },
  touch: files('Creating', 'Creating a file', 'Creating files', '-d -r -t --date --reference'),
  chmod: (a) => named('Changing permissions on', positional(a, '--reference').slice(1), 'Changing permissions', 'Changing permissions'),
  chown: (a) => named('Changing the owner of', positional(a, '--reference').slice(1), 'Changing the owner', 'Changing the owner'),
  ln: () => 'Linking files',
  stat: files('Checking', 'Checking a file', 'Checking files', '-c --format --printf'),
  file: files('Checking', 'Checking a file', 'Checking files'),
  tar: (a) => {
    const flags = a.map((w) => w.text).find((t) => /^-?[A-Za-z]+$/.test(t)) ?? '';
    const f = a.findIndex((w) => /^-?[A-Za-z]*f$/.test(w.text) || w.text === '--file');
    const archive = f >= 0 ? [a[f + 1]].filter(Boolean) : [];
    if (/x/.test(flags) || a.some((w) => w.text === '--extract')) return named('Unpacking', archive, 'Unpacking files', 'Unpacking files');
    if (/t/.test(flags) || a.some((w) => w.text === '--list')) return named('Listing', archive, 'Listing an archive', 'Listing an archive');
    return 'Packing files';
  },
  unzip: (a) => named('Unpacking', positional(a, '-d -x').slice(0, 1), 'Unpacking files', 'Unpacking files'),
  zip: () => 'Packing files',
  gzip: (a) => (hasFlag(a, '-d') ? 'Unpacking files' : 'Packing files'),
  gunzip: () => 'Unpacking files',
  // packages, tests, builds
  npm: (a, s, all, at, d) => pkg(a, s, all, at, d, 'npm'),
  pnpm: (a, s, all, at, d) => pkg(a, s, all, at, d, 'pnpm'),
  yarn: (a, s, all, at, d) => pkg(a, s, all, at, d, 'yarn'),
  bun: (a, s, all, at, d) => pkg(a, s, all, at, d, 'bun'),
  npx: (a, s, all, at, d) => inner(a.filter((w) => !/^(-y|--yes|--no-install|-q|--quiet)$/.test(w.text)), s, all, at, d),
  bunx: (a, s, all, at, d) => inner(a, s, all, at, d),
  pip: (a) => pip(a),
  pip3: (a) => pip(a),
  pipx: (a) => (positional(a)[0]?.text === 'run' ? 'Running a tool' : pip(a)),
  uv: (a, s, all, at, d) => {
    const pos = positional(a, '--python -p --with');
    const sub = pos[0]?.text;
    if (sub === 'run') return inner(a.slice(a.indexOf(pos[0]) + 1).filter((w) => !w.text.startsWith('-')), s, all, at, d) ?? 'Running a script';
    if (sub === 'pip') return pip(a.slice(a.indexOf(pos[0]) + 1));
    if (sub === 'add' || sub === 'sync' || sub === 'install' || sub === 'lock') return 'Installing packages';
    if (sub === 'venv') return 'Making a Python environment';
    return null;
  },
  poetry: (a, s, all, at, d) => {
    const pos = positional(a);
    if (pos[0]?.text === 'run') return inner(pos.slice(1), s, all, at, d);
    return /^(install|add|update|lock)$/.test(pos[0]?.text ?? '') ? 'Installing packages' : null;
  },
  pipenv: (a, s, all, at, d) => {
    const pos = positional(a);
    return pos[0]?.text === 'run' ? inner(pos.slice(1), s, all, at, d) : 'Installing packages';
  },
  bundle: (a, s, all, at, d) => {
    const pos = positional(a);
    return pos[0]?.text === 'exec' ? inner(pos.slice(1), s, all, at, d) : 'Installing packages';
  },
  gem: (a) => (positional(a)[0]?.text === 'install' ? 'Installing packages' : null),
  composer: (a) => (/^(install|require|update)$/.test(positional(a)[0]?.text ?? '') ? 'Installing packages' : null),
  conda: (a) => (/^(install|create|update)$/.test(positional(a)[0]?.text ?? '') ? 'Installing packages' : null),
  apt: sys({ install: 'Installing packages', update: 'Updating packages', upgrade: 'Updating packages', remove: 'Removing packages', purge: 'Removing packages', search: 'Looking up packages', list: 'Checking packages', show: 'Looking up a package' }, 'Managing packages'),
  'apt-get': sys({ install: 'Installing packages', update: 'Updating packages', upgrade: 'Updating packages', 'dist-upgrade': 'Updating packages', remove: 'Removing packages', purge: 'Removing packages', autoremove: 'Removing packages' }, 'Managing packages'),
  dnf: sys({ install: 'Installing packages', update: 'Updating packages', upgrade: 'Updating packages', remove: 'Removing packages' }, 'Managing packages'),
  yum: sys({ install: 'Installing packages', update: 'Updating packages', remove: 'Removing packages' }, 'Managing packages'),
  apk: sys({ add: 'Installing packages', update: 'Updating packages', upgrade: 'Updating packages', del: 'Removing packages' }, 'Managing packages'),
  brew: sys({ install: 'Installing packages', update: 'Updating packages', upgrade: 'Updating packages', uninstall: 'Removing packages', list: 'Checking packages', info: 'Looking up a package' }, 'Using Homebrew'),
  pacman: (a) => (a.some((w) => /^-S/.test(w.text)) ? 'Installing packages' : 'Managing packages'),
  snap: sys({ install: 'Installing packages', refresh: 'Updating packages', remove: 'Removing packages' }, 'Managing packages'),
  cargo: (a) => {
    const sub = positional(a, '--manifest-path -p --package --target -j --jobs --features')[0]?.text ?? '';
    const C: Record<string, string> = {
      test: 'Running tests',
      nextest: 'Running tests',
      build: 'Building',
      check: 'Checking the code',
      clippy: 'Checking the code',
      run: 'Running the app',
      fmt: 'Formatting the code',
      add: 'Installing packages',
      install: 'Installing packages',
      update: 'Updating packages',
      doc: 'Building the docs',
      bench: 'Running benchmarks',
      clean: 'Cleaning up the build',
    };
    return Object.prototype.hasOwnProperty.call(C, sub) ? C[sub] : 'Running cargo';
  },
  go: (a) => {
    const pos = positional(a, '-o -run -tags -C');
    const sub = pos[0]?.text ?? '';
    if (sub === 'test') return 'Running tests';
    if (sub === 'build' || sub === 'install') return 'Building';
    if (sub === 'run') return named('Running', pos.slice(1, 2), 'Running the app', 'Running the app');
    if (sub === 'get' || sub === 'mod') return 'Installing packages';
    if (sub === 'vet') return 'Checking the code';
    if (sub === 'fmt') return 'Formatting the code';
    return 'Running go';
  },
  make: (a) => {
    const t = positional(a, '-C -f -j -l -o -W --directory --file --jobs').find((w) => !w.text.includes('='))?.text ?? '';
    if (/^(test|tests|check|e2e)$/.test(t)) return 'Running tests';
    if (/^(install)$/.test(t)) return 'Installing';
    if (/^(clean)$/.test(t)) return 'Cleaning up the build';
    if (/^(lint|fmt|format)$/.test(t)) return t === 'lint' ? 'Checking the code' : 'Formatting the code';
    return 'Building';
  },
  cmake: () => 'Building',
  ninja: () => 'Building',
  gradle: (a) => (positional(a).some((w) => /^(test|check)$/.test(w.text)) ? 'Running tests' : 'Building'),
  gradlew: (a) => (positional(a).some((w) => /^(test|check)$/.test(w.text)) ? 'Running tests' : 'Building'),
  mvn: (a) => (positional(a).some((w) => w.text === 'test') ? 'Running tests' : 'Building'),
  vite: (a) => (positional(a)[0]?.text === 'build' ? 'Building' : 'Starting the app'),
  next: (a) => (positional(a)[0]?.text === 'build' ? 'Building' : 'Starting the app'),
  webpack: () => 'Building',
  esbuild: () => 'Building',
  tsc: () => 'Checking types',
  pytest: () => 'Running tests',
  'py.test': () => 'Running tests',
  jest: () => 'Running tests',
  vitest: () => 'Running tests',
  mocha: () => 'Running tests',
  playwright: (a) => (positional(a)[0]?.text === 'install' ? 'Installing browsers' : 'Running tests'),
  rspec: () => 'Running tests',
  phpunit: () => 'Running tests',
  tox: () => 'Running tests',
  nox: () => 'Running tests',
  eslint: () => 'Checking the code',
  ruff: (a) => (positional(a)[0]?.text === 'format' ? 'Formatting the code' : 'Checking the code'),
  flake8: () => 'Checking the code',
  pylint: () => 'Checking the code',
  mypy: () => 'Checking types',
  pyright: () => 'Checking types',
  shellcheck: () => 'Checking the code',
  prettier: () => 'Formatting the code',
  black: () => 'Formatting the code',
  rustfmt: () => 'Formatting the code',
  gofmt: () => 'Formatting the code',
  // running a script
  python: interp('Python'),
  python3: interp('Python'),
  node: interp('JavaScript'),
  deno: (a, s, all, at, d) => (positional(a)[0]?.text === 'run' ? interp('JavaScript')(positional(a).slice(1), s, all, at, d) : interp('JavaScript')(a, s, all, at, d)),
  ruby: interp('Ruby'),
  perl: interp('Perl'),
  php: interp('PHP'),
  tsx: interp('JavaScript'),
  'ts-node': interp('JavaScript'),
  // network
  curl: (a) => {
    const h = fetchHost(a, CURL_TAKES);
    return h ? `Fetching ${h}` : 'Fetching a page';
  },
  wget: (a) => {
    const h = fetchHost(a, WGET_TAKES);
    return h ? `Downloading from ${h}` : 'Downloading a file';
  },
  http: (a) => {
    const h = fetchHost(a, '-a --auth');
    return h ? `Fetching ${h}` : 'Fetching a page';
  },
  ping: (a) => {
    const h = positional(a, '-c -i -W -w -s -t -I').map((w) => (w.dynamic ? null : hostOf(w.text)))[0];
    return h ? `Checking ${h} is reachable` : 'Checking a connection';
  },
  dig: (a) => {
    const h = a.map((w) => (w.dynamic || /^[-+@]/.test(w.text) ? null : hostOf(w.text))).find((x) => x && x.includes('.'));
    return h ? `Looking up ${h}` : 'Looking up a name';
  },
  nslookup: (a) => {
    const h = positional(a)[0];
    const host = h && !h.dynamic ? hostOf(h.text) : null;
    return host ? `Looking up ${host}` : 'Looking up a name';
  },
  host: (a) => {
    const h = positional(a, '-t -W')[0];
    const host = h && !h.dynamic ? hostOf(h.text) : null;
    return host ? `Looking up ${host}` : 'Looking up a name';
  },
  nc: () => 'Checking a connection',
  telnet: () => 'Checking a connection',
  ss: () => 'Checking open ports',
  netstat: () => 'Checking open ports',
  lsof: () => 'Checking open files',
  scp: (a) => copyRemote(a),
  rsync: (a) => copyRemote(a),
  // services and processes
  systemctl: (a) => {
    const pos = positional(a, '-H --host -M --machine -n --lines -o --output -p --property -t --type --state');
    const verb = pos[0]?.text ?? '';
    if (verb === 'daemon-reload') return 'Reloading services';
    if (/^list-/.test(verb) || verb === '') return 'Listing services';
    return service(verb, pos.slice(1)) ?? 'Using systemctl';
  },
  service: (a) => {
    const pos = positional(a);
    return (pos.length >= 2 && service(pos[1].text, [pos[0]])) || 'Using service';
  },
  journalctl: (a) => {
    for (let i = 0; i < a.length; i++) {
      const t = a[i].text;
      const u = t === '-u' || t === '--unit' ? a[i + 1] : t.startsWith('--unit=') ? { text: t.slice(7), dynamic: a[i].dynamic } : /^-u.+/.test(t) ? { text: t.slice(2), dynamic: a[i].dynamic } : null;
      if (u) {
        const n = unitName(u);
        return n ? `Reading logs for ${n}` : 'Reading logs';
      }
    }
    return 'Reading logs';
  },
  docker: (a) => docker(a, false),
  podman: (a) => docker(a, false),
  'docker-compose': (a) => docker(a, true),
  pm2: (a) => {
    const pos = positional(a);
    const sub = pos[0]?.text ?? '';
    if (sub === 'logs') return 'Reading logs';
    if (/^(list|ls|status|l|jlist)$/.test(sub)) return 'Checking running apps';
    const v = sub === 'reload' ? 'Restarting' : SERVICE_VERB[sub];
    if (!v) return 'Using pm2';
    const n = pos.length === 2 ? nameOf(pos[1]) : null;
    return n ? `${v} ${n}` : `${v} an app`;
  },
  kubectl: (a) => {
    const sub = positional(a, '-n --namespace --context -o --output -l --selector -f --filename -c --container')[0]?.text ?? '';
    if (sub === 'get' || sub === 'describe' || sub === 'top') return 'Checking the cluster';
    if (sub === 'logs') return 'Reading logs';
    if (sub === 'apply' || sub === 'patch' || sub === 'edit' || sub === 'set') return 'Changing the cluster';
    if (sub === 'rollout') return 'Restarting an app';
    if (sub === 'delete') return 'Deleting from the cluster';
    return 'Using kubectl';
  },
  kill: () => 'Stopping a program',
  pkill: (a) => {
    if (hasFlag(a, '-f')) return 'Stopping a program';
    const n = positional(a, '-u -U -g -G -s -t --signal')[0];
    const p = n ? programOf(n) : null;
    return p ? `Stopping ${p}` : 'Stopping a program';
  },
  killall: (a) => {
    const n = positional(a, '-u -s --signal')[0];
    const p = n ? programOf(n) : null;
    return p ? `Stopping ${p}` : 'Stopping a program';
  },
  ps: () => 'Checking running programs',
  pgrep: () => 'Checking running programs',
  top: () => 'Checking running programs',
  htop: () => 'Checking running programs',
  uptime: () => 'Checking the system',
  uname: () => 'Checking the system',
  hostname: () => 'Checking the system',
  whoami: () => 'Checking the system',
  id: () => 'Checking the system',
  nproc: () => 'Checking the system',
  lscpu: () => 'Checking the system',
  pwd: () => 'Checking the current folder',
  date: () => 'Checking the time',
  env: () => 'Checking settings',
  printenv: () => 'Checking settings',
  which: (a) => installed(a),
  whereis: (a) => installed(a),
  type: (a) => installed(a),
  crontab: (a) => (hasFlag(a, '-l') ? 'Checking scheduled jobs' : 'Changing scheduled jobs'),
  sleep: (a) => {
    const m = /^(\d+(?:\.\d+)?)([smhd]?)$/.exec(a[0]?.text ?? '');
    if (!m || a[0].dynamic) return 'Waiting';
    const unit = { '': 'second', s: 'second', m: 'minute', h: 'hour', d: 'day' }[m[2]] ?? 'second';
    return `Waiting ${m[1]} ${unit}${m[1] === '1' ? '' : 's'}`;
  },
  wait: () => 'Waiting',
  sqlite3: () => 'Querying a database',
  psql: () => 'Querying a database',
  mysql: () => 'Querying a database',
  'redis-cli': () => 'Querying a database',
  ffmpeg: () => 'Converting media',
  ffprobe: () => 'Checking a media file',
  test: (a) => named('Checking', positional(a).slice(-1), 'Checking something', 'Checking something'),
  '[': (a) => named('Checking', positional(a.filter((w) => w.text !== ']')).slice(-1), 'Checking something', 'Checking something'),
  '[[': (a) => named('Checking', positional(a.filter((w) => w.text !== ']]')).slice(-1), 'Checking something', 'Checking something'),
};

function installed(a: Word[]): string {
  const p = positional(a).length === 1 ? programOf(positional(a)[0]) : null;
  return p ? `Checking ${p} is installed` : 'Checking what is installed';
}

/** scp/rsync: "Copying files to 203.0.113.66" when a side is "host:path"; never the path. */
function copyRemote(a: Word[]): string {
  const pos = positional(a, '-P -i -o -F -e --rsh -l -S --exclude --include --filter --port');
  const remote = (w: Word | undefined) => {
    if (!w || w.dynamic) return null;
    const m = /^([^/:]+):/.exec(w.text);
    return m ? hostOf(m[1]) : null;
  };
  const to = remote(pos[pos.length - 1]);
  if (to) return `Copying files to ${to}`;
  const from = pos.slice(0, -1).map(remote).find(Boolean);
  return from ? `Copying files from ${from}` : 'Copying files';
}

// ---- the command ----

/** Words that only set things up for the next command: a row never says only these when more follows. */
const SETUP = new Set(['cd', 'pushd', 'popd', 'export', 'set', 'unset', 'source', '.', 'true', ':', 'alias', 'umask', 'shopt', 'trap', 'exit', 'return', 'local', 'declare', 'readonly', 'ulimit']);
/** Shell words that open or close a block: skipped (for, case, done) or stripped (then, do, else). */
const SKIP_KEYWORDS = new Set(['for', 'select', 'case', 'esac', 'done', 'fi', '}', 'function', ';;']);
const STRIP_KEYWORDS = new Set(['do', 'then', 'else', 'elif', 'if', 'while', 'until', '{', '!', 'time']);
/** Programs that run the next words as the command, with the flags that take a value. */
const PREFIX: Record<string, string> = {
  sudo: '-u -g -h -p -C -D -r -t -U -T --user --group --host --prompt --close-from --chdir',
  doas: '-u -C',
  env: '-u -C -S --unset --chdir --split-string',
  nohup: '',
  nice: '-n --adjustment',
  ionice: '-c -n -p -t',
  timeout: '-s -k --signal --kill-after',
  exec: '-a',
  command: '',
  builtin: '',
  stdbuf: '-i -o -e',
  xargs: '-I -i -n -P -L -l -d -E -s -a --max-args --max-procs --replace --delimiter --arg-file',
  sshpass: '-p -f -d -e -P',
  caffeinate: '-t -w',
  watch: '-n -d --interval --differences',
  chronic: '',
  flock: '-w -E --timeout --conflict-exit-code',
  setsid: '',
  unbuffer: '',
};

/** Leading `NAME=value` words, `sudo`/`env`/`timeout N`/`xargs` and their flags, and block keywords, taken off. */
function stripPrefix(words: Word[]): Word[] {
  let w = words;
  for (let guard = 0; guard < 12 && w.length; guard++) {
    const t = w[0].text;
    if (/^[A-Za-z_][A-Za-z0-9_]*\+?=/.test(t)) {
      w = w.slice(1);
      continue;
    }
    if (STRIP_KEYWORDS.has(t) && !w[0].dynamic) {
      w = w.slice(1);
      continue;
    }
    const p = programOf(w[0]);
    if (p && Object.prototype.hasOwnProperty.call(PREFIX, p) && !(p === 'command' && w[1]?.text === '-v')) {
      const takes = new Set(PREFIX[p].split(' ').filter(Boolean));
      let i = 1;
      while (i < w.length) {
        const a = w[i].text;
        if (a === '--') {
          i++;
          break;
        }
        if (a.startsWith('-') && a.length > 1) {
          i += !a.includes('=') && takes.has(a) ? 2 : 1;
          continue;
        }
        if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(a) && (p === 'env' || p === 'sudo')) {
          i++;
          continue;
        }
        break;
      }
      // timeout's and flock's first word is the duration or the lock file
      if ((p === 'timeout' || p === 'flock') && i < w.length) i++;
      w = w.slice(i);
      continue;
    }
    break;
  }
  return w;
}

/** ssh: the host, and the command it runs there as its own words (re-read as a command). */
function sshParts(args: Word[]): { host: string | null; rest: Word[] } {
  for (let i = 0; i < args.length; i++) {
    const t = args[i].text;
    if (t === '--') return { host: args[i + 1] && !args[i + 1].dynamic ? hostOf(args[i + 1].text.replace(/^ssh:\/\//, '')) : null, rest: args.slice(i + 2) };
    if (t.startsWith('-') && t.length > 1) {
      if (t.length === 2 && 'BbcDEeFIiJLlmOoPpQRSWw'.includes(t[1])) i++;
      continue;
    }
    return { host: args[i].dynamic ? null : hostOf(args[i].text.replace(/^ssh:\/\//, '').replace(/\/.*$/, '')), rest: args.slice(i + 1) };
  }
  return { host: null, rest: [] };
}

const SHELLS = new Set(['bash', 'sh', 'zsh', 'dash', 'ksh', 'fish']);

/** One segment in words; null when it is only setup (cd, export) or says nothing readable. */
function analyse(seg: Segment, all: Segment[], at: number, depth: number): string | ShellWords | null {
  if (depth > 4) return null;
  const words = stripPrefix(seg.words);
  if (!words.length) return null;
  const first = words[0];
  if (SKIP_KEYWORDS.has(first.text)) return null;
  const prog = programOf(first);
  const args = words.slice(1);
  if (!prog) return null;
  const p = prog.toLowerCase();
  if (SETUP.has(p)) return null;
  if (p === 'ssh' || p === 'mosh') {
    const { host, rest } = sshParts(args);
    // ssh joins the words with spaces and the far shell reads them again, so do the same.
    const there = rest.length ? describe(rest.map((w) => w.text).join(' '), depth + 1) : null;
    if (!host) return there?.host ? there : there ? { text: there.text, host: null } : 'Connecting to a server';
    if (!rest.length) return { text: `Connecting to ${host}`, host };
    if (there?.host) return there;
    return { text: `${there && there.text !== 'Running a command' ? there.text : 'Running a command'} on ${host}`, host };
  }
  if (SHELLS.has(p)) {
    const c = args.findIndex((w) => /^-[a-zA-Z]*c[a-zA-Z]*$/.test(w.text));
    if (c >= 0 && args[c + 1]) return describe(args[c + 1].text, depth + 1);
    const f = positional(args, '-o +o -O +O');
    if (f.length) return named('Running', [f[0]], 'Running a script', 'Running a script');
    return null;
  }
  if (args.length === 1 && /^(--version|-V)$/.test(args[0].text)) return `Checking the ${prog} version`;
  if (Object.prototype.hasOwnProperty.call(RULES, p)) {
    const r = RULES[p](args, seg, all, at, depth);
    if (r) return r;
  }
  if (p === 'command' && args[0]?.text === '-v') return installed(args.slice(1));
  return `Running ${prog}`;
}

/** Low-value rows that a later, more telling command in the same line beats ("echo ---; git status"). */
const WEAK = /^(Printing a message|Checking the current folder|Checking the time|Checking settings|Waiting)$/;

function describe(cmd: string, depth = 0): ShellWords | null {
  const segs = segments(cmd);
  let weak: ShellWords | null = null;
  for (let i = 0; i < segs.length; i++) {
    // The right side of a pipe only shapes the left side's output ("git log | head").
    if (i > 0 && segs[i - 1].next === '|') continue;
    const r = analyse(segs[i], segs, i, depth);
    if (!r) continue;
    const w = typeof r === 'string' ? { text: r, host: null } : r;
    if (WEAK.test(w.text) && !w.host) {
      weak ??= w;
      continue;
    }
    return w;
  }
  return weak;
}

/**
 * A shell command in words: "Checking git status", "Reading config.toml", "Restarting trailhead-shop on
 * 203.0.113.66", "Running deploy.sh". An unknown program reads "Running <program>"; only when nothing plain is
 * readable, "Running a command". `cut` means the command was cut short (a title's "…"), so its last word may be
 * half a word and is dropped.
 */
export function shellWords(cmd: string, cut = false): ShellWords {
  let c = cmd;
  if (cut) c = c.replace(/…$/, '').replace(/\S*$/, '');
  const w = describe(c);
  return w ? { text: cap(w.text), host: w.host } : { text: 'Running a command', host: null };
}
