// Sign-ins that can never work from here (Mira's design check for #574). The server's start call
// returns one of these codes; retrying can't help, so the button goes and one next step takes its place.

export type SignInDeadEndCode = 'no_client_registration' | 'cimd_needs_https';

/** Where the copy shows: the Tool servers screen, or a bot's suggestion card in the chat. */
export type SignInSurface = 'servers' | 'card';

export interface SignInDeadEnd {
  code: SignInDeadEndCode;
  /** The rose error line. */
  error: string;
  /** The grey line that replaces Retry, Connect or "Try signing in again". */
  next: string;
}

export const isSignInDeadEndCode = (code: unknown): code is SignInDeadEndCode =>
  code === 'no_client_registration' || code === 'cimd_needs_https';

const ERROR: Record<SignInDeadEndCode, string> = {
  no_client_registration: 'This server only lets in apps that were set up with it in advance, so signing in from here can’t work.',
  cimd_needs_https: 'This server’s sign-in only works from localhost or an https address, not from a plain http address like this one.',
};

const LOCALHOST = 'Open this app in a browser on the computer it runs on, at localhost, and sign in there. Your bots here use that sign-in too.';

const next = (code: SignInDeadEndCode, name: string, surface: SignInSurface) => {
  if (code === 'cimd_needs_https') return LOCALHOST;
  return surface === 'servers'
    ? `If ${name} gives you an API key, edit this server and add the key under Headers instead.`
    : `If ${name} gives you an API key, add it under Headers in Settings › Apps & accounts › Advanced › Tool servers.`;
};

/**
 * The copy for a sign-in that can't work, or null for any other code (or none): then the screen keeps
 * today's error and its Retry. `name` is the server's name, or the suggestion's title on the card.
 */
export const signInDeadEnd = (code: unknown, name: string, surface: SignInSurface): SignInDeadEnd | null => {
  if (!isSignInDeadEndCode(code)) return null;
  const error = surface === 'card' ? `Couldn’t start the sign-in. ${ERROR[code]}` : ERROR[code];
  return { code, error, next: next(code, name, surface) };
};
