# Package-manager listings

The source copies of what kwant submits to package managers. A change here is reviewed in this repo
first; `.github/workflows/packaging.yml` installs and checks each one on pull requests.

| Folder | Goes to | Name | Updates on each release |
|---|---|---|---|
| `homebrew/kwnt.rb` | the tap [alexsergivan/homebrew-kwant](https://github.com/alexsergivan/homebrew-kwant) as `Formula/kwnt.rb` | `brew install alexsergivan/kwant/kwnt` (the `kwnt` server and CLI, built from the `mac-vX.Y.Z` tag) | the maintainers' `registry_bump.sh <version>` rewrites its url and sha256 |
| `winget/manifests/a/alexsergivan/kwant/<version>/` | [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) | `winget install alexsergivan.kwant` (the Windows app, beta) | the maintainers' `registry_bump.sh <version>` (komac), after the release is published |
| `scoop/kwant.json` | [ScoopInstaller/Extras](https://github.com/ScoopInstaller/Extras) | `scoop install extras/kwant` (the Windows app, beta) | Scoop's Excavator, from the manifest's `checkver` and `autoupdate` |

The copies here keep the version they were first checked with; the live listings move on their own.

kwnt isn't in homebrew-core: its contributing rules want every review comment answered by a person
without AI, so that submission is for a maintainer to make by hand. The formula is written to pass
`brew audit --new --strict --online`, which this repo's CI runs.
To check a change, open a pull request that touches `packaging/**`.
