// The bundled "What's new" files and CHANGELOG sections (vite.config.ts builds the virtual module).
import { changelog, entries } from 'virtual:kwant-whatsnew';
import { sortEntries, type WhatsNewEntry } from './whatsNewSheet';

/** Every good entry, newest first. The first is the version the sheet shows after an update. */
export const WHATS_NEW: WhatsNewEntry[] = sortEntries(entries);
export const latestWhatsNew = (): WhatsNewEntry | null => WHATS_NEW[0] ?? null;
export const whatsNewFor = (version: string): WhatsNewEntry | null => WHATS_NEW.find((e) => e.version === version) ?? null;
export const changelogFor = (version: string): string | null => changelog[version] ?? null;
