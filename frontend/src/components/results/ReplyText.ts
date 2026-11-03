import React from 'react';

/**
 * The reply's own markdown text, set by MarkdownRenderer around its blocks. A block that would repeat a
 * sentence the reply already shows (a chart's summary caption) leaves it out; see `repeatsReply`.
 */
export const ReplyText = React.createContext<string>('');
