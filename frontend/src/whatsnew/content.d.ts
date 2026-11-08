// Built by the kwnt-whats-new plugin in vite.config.ts from src/whatsnew/*.json and ../CHANGELOG.md.
declare module 'virtual:kwant-whatsnew' {
  /** Each src/whatsnew/<version>.json as written. */
  export const entries: unknown[];
  /** Each of those versions' "## [Mac X.Y.Z]" CHANGELOG section, without its heading (null when missing). */
  export const changelog: Record<string, string | null>;
}
