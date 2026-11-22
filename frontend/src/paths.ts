// 33a: folders in plain words. The frontend has no HOME from the server, so a home is recognised by
// its shape: /home/<user>, /Users/<user> or <drive>:\Users\<user> (either slash).

const HOME = /^(?:\/home\/[^/]+|\/Users\/[^/]+|[A-Za-z]:[\\/]Users[\\/][^\\/]+)(?=$|[\\/])/;

/**
 * A folder under a home as `~/rest` (`~\rest` on Windows, keeping its slash; `~` for the home
 * itself). Anything else (`/tmp/x`, `/root/x`, `/var/…`) comes back as it is. For display only:
 * the exact folder stays in the element's `title`.
 */
export function plainPath(path: string): string {
  const m = HOME.exec(path);
  return m ? `~${path.slice(m[0].length)}` : path;
}
