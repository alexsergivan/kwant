// 30e (autonomy/details-no-echo-reasoning/brief.md): a CLI reply saves "Executed via claude CLI · This machine
// [Effort: medium]" as its reasoning (src/drivers/cli.rs executed_via). It repeats Details' "Ran on" row and reads like
// thinking to open when there is none, so Details hides it and keeps only the effort, in words, next to Ran on.

/** The whole of a CLI run line: one line, nothing before or after. */
const CLI_RUN_LINE = /^Executed via .+ CLI · .+ \[Effort: ([^\]]+)\]$/;

/** The run line's effort when the reasoning is only that line, else null (real reasoning, or more than one line). */
export function cliRunLineEffort(reasoning: string | null | undefined): string | null {
  const text = reasoning?.trim();
  if (!text || text.includes('\n')) return null;
  const m = CLI_RUN_LINE.exec(text);
  return m ? m[1].trim() : null;
}

/** Whether the reasoning is only the CLI run line, so there is nothing to open. */
export const isCliRunLine = (reasoning: string | null | undefined): boolean => cliRunLineEffort(reasoning) !== null;

/** The effort in plain words for "Ran on": "medium effort". Null when the line didn't carry one. */
export function effortWords(reasoning: string | null | undefined): string | null {
  const effort = cliRunLineEffort(reasoning);
  return effort ? `${effort.toLowerCase()} effort` : null;
}
