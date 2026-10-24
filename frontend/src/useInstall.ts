import { useSyncExternalStore } from 'react';
import { getInstallState, InstallState, subscribeInstallState } from './pwa';

/** The browser's install offer and whether kwant is installed, kept since page load. */
export const useInstall = (): InstallState => useSyncExternalStore(subscribeInstallState, getInstallState, getInstallState);
