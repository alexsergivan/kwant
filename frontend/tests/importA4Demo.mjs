// Demo data for A4 (0.8.0 import UI), shaped like release-080/import-contract.md. Shared by
// importA4.test.mjs and the screenshot script. Made-up names only; no real accounts.
export const skill = (name) => ({ name, description: `${name} skill`, body: `# ${name}\n` });

export const localList = (over = {}) => ({
  available: true,
  sources: [
    { id: 'openclaw', kind: 'openclaw', label: 'OpenClaw', path: '~/.openclaw/workspace', bots: 1, skills: 3, memories: 2, names: ['seo', 'support', 'triage'], too_large: false },
    { id: 'claude-skills', kind: 'claude-skills', label: 'Claude Code skills', path: '~/.claude/skills', bots: 0, skills: 7, memories: 0, names: ['changelog', 'deploy', 'fix', 'plan', 'review', 'seo-audit', 'triage'], too_large: false },
    { id: 'codex', kind: 'codex', label: 'Codex', path: '~/.codex', bots: 1, skills: 40, memories: 0, names: [], too_large: true },
  ],
  ...over,
});

export const bot = (ref, name, title) => ({ ref, name, title, persona_prompt: `You are ${name}. Keep answers short and kind.`, memories: ['Likes tidy notes.'], skills: [skill('triage')], routines: [] });

export const localPreview = () => ({
  bundle: {
    format: 'kwant.bundle/1',
    source: { kind: 'local', label: 'this computer', exported_at: null },
    bots: [bot('molty', 'Molty', 'Support lead')],
    teams: [],
    skills: ['changelog', 'deploy', 'fix', 'plan', 'review', 'seo-audit', 'triage'].map(skill),
    omitted: ['openclaw.json and credentials/: never opened, so no keys or tokens', 'Daily logs in memory/: 14 files'],
  },
  warnings: [],
  dropped: [],
  problems: [],
  renames: [],
  can_apply: true,
});

const SKILLS12 = ['billing-faq', 'escalate', 'first-reply', 'refunds', 'tone-check', 'triage', 'macros', 'sla-watch', 'handover', 'csat', 'bug-report', 'close-ticket'];

export const githubInfo = (over = {}) => ({
  owner: 'example-team',
  repo: 'support-skills',
  ref: 'HEAD',
  path: null,
  commit: '3f2a91c08d5e4b7a9c1f2e3d4c5b6a7980716253',
  licence: { spdx: 'MIT', name: 'MIT License', file: 'LICENSE' },
  ...over,
});

export const githubPreview = (over = {}) => {
  const github = githubInfo(over);
  const lic = github.licence ? github.licence.spdx ?? github.licence.name : 'no licence';
  return {
    bundle: {
      format: 'kwant.bundle/1',
      source: { kind: 'github', label: `${github.owner}/${github.repo}`, exported_at: null },
      bots: [],
      teams: [],
      skills: SKILLS12.map(skill),
      omitted: [`From github.com/${github.owner}/${github.repo} (${lic})`, 'scripts/: 6 files that run code'],
    },
    warnings: github.licence ? [] : [{ message: 'This repo has no licence file. You may not have the right to reuse or share these skills.' }],
    dropped: [],
    problems: [],
    renames: [],
    can_apply: true,
    github,
  };
};

export const folderPreview = () => ({
  bundle: {
    format: 'kwant.bundle/1',
    source: { kind: 'openclaw', label: 'OpenClaw', exported_at: null },
    bots: [bot('molty', 'Molty', 'Support lead')],
    teams: [],
    skills: [],
    omitted: ["Files in the zip that weren't read: notes.txt"],
  },
  warnings: [],
  dropped: [],
  problems: [],
  renames: [],
  can_apply: true,
});

export const applied = (preview) => ({
  import: {
    id: 'imp-demo',
    space_id: 'home',
    source_kind: preview.bundle.source.kind,
    status: 'applied',
    created: {
      kwants: preview.bundle.bots.map((b, i) => ({ ref: b.ref, id: `k${i}`, name: b.name })),
      skills: [...preview.bundle.bots.flatMap((b) => b.skills.map((s) => s.name)), ...preview.bundle.skills.map((s) => s.name)],
      routines: [],
      teams: [],
      channels: [],
      memories: 0,
    },
    created_at: '2026-11-09T07:00:00Z',
    undone_at: null,
  },
  warnings: [],
  dropped: [],
  renames: [],
});

/** A picked folder as `<input webkitdirectory>` gives it (relative paths include the folder). */
export const folderFiles = (name = 'molty-workspace') => [
  [`${name}/SOUL.md`, 'Be kind.'],
  [`${name}/IDENTITY.md`, '- **Name:** Molty'],
  [`${name}/AGENTS.md`, 'Work in small steps.'],
  [`${name}/skills/triage/SKILL.md`, '---\nname: triage\n---\n'],
  [`${name}/notes.txt`, 'random'],
  [`${name}/.env`, 'OPENAI_API_KEY=sk-demo-not-real'],
  [`${name}/.git/HEAD`, 'ref: refs/heads/main'],
  [`${name}/openclaw.json`, '{"apiKey":"sk-demo-not-real"}'],
];
