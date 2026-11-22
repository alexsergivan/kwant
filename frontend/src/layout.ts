/** Below 768 px the app is one column (a list or a chat, with a tab bar) and the sidebar is not shown. */
export const PHONE_QUERY = '(max-width: 767px)';

/** Tailwind's `sm` (640 px and up): where a chat row moves its avatar from the name line to the gutter (33q). */
export const SM_QUERY = '(min-width: 640px)';

/** The phone layout is on: no sidebar, so nothing on screen should be placed from it. */
export const isPhoneLayout = (): boolean => typeof window !== 'undefined' && !!window.matchMedia?.(PHONE_QUERY).matches;

/** A touch screen: the finger is the pointer. With PHONE_QUERY, the phone layout on a phone. */
export const TOUCH_QUERY = '(pointer: coarse)';
