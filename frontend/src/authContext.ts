// Who this browser is to the server, shared by AuthGate and anything that reads it. Kept apart from
// AuthGate so reading it doesn't load the gate (which watches every fetch for a 401 on import).
import { createContext, useContext } from 'react';

export interface AuthContextValue {
  /** True when this browser needs the API token (Docker, a VPS reached directly). */
  authRequired: boolean;
  /** This server lets new browsers in from a signed-in one, and lists signed-in browsers. */
  deviceLogin: boolean;
  signOut: () => Promise<void>;
}

export const AuthContext = createContext<AuthContextValue>({ authRequired: false, deviceLogin: false, signOut: async () => undefined });

export const useAuth = () => useContext(AuthContext);
