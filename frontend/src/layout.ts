/** Below 768 px the app is one column (a list or a chat, with a tab bar) and the sidebar is not shown. */
export const PHONE_QUERY = '(max-width: 767px)';

/** The phone layout is on: no sidebar, so nothing on screen should be placed from it. */
export const isPhoneLayout = (): boolean => typeof window !== 'undefined' && !!window.matchMedia?.(PHONE_QUERY).matches;

/** A touch screen: the finger is the pointer. With PHONE_QUERY, the phone layout on a phone. */
export const TOUCH_QUERY = '(pointer: coarse)';
