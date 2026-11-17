# kwant in self-hosted app stores and on Render

Each listing runs the released kwnt image alone (no Laya, no bots' browser), pinned to the release
version, with `/data` on the store's persistent storage. Bots are then picked by keyword, VM mode is
off, and there are no AI CLIs inside the image, so you add an API key in the app.

| Where | File | Data | Sign-in token | Store login in front |
|---|---|---|---|---|
| CasaOS / ZimaOS | [`deploy/casaos/Kwnt/docker-compose.yml`](../deploy/casaos/Kwnt/docker-compose.yml) | `/DATA/AppData/kwnt/data` | kwnt's own: run `kwnt token --raw` in the app's **Terminal** | none |
| Umbrel | [`deploy/umbrel/kwant/`](../deploy/umbrel/kwant) | `${APP_DATA_DIR}/data` | the password Umbrel shows (`APP_PASSWORD`, `deterministicPassword: true`) | Umbrel login |
| Coolify | [`deploy/coolify/kwnt.yaml`](../deploy/coolify/kwnt.yaml) | named volume `kwnt-data` | `SERVICE_PASSWORD_64_KWNT` from the service's environment variables | none (HTTPS from Coolify's proxy) |
| Render | [`render.yaml`](../render.yaml) | 1 GB disk on `/data` (**paid instance only**) | `KWNT_API_TOKEN` (Render's `generateValue`) from the **Environment** page | none |

Every listing sets `KWNT_SIGNIN_HINT`, one line the sign-in screen shows before anyone signs in, so
the first sign-in needs nothing but the browser and the store's own page. Images before the release
that added the hint ignore it; their token box is under **Other setups**.

## Why each file looks the way it does

- **kwnt runs as uid 10001.** A named volume inherits that from the image (Coolify). A bind mount the
  store creates belongs to root, so CasaOS runs a one-shot `kwnt-init` service (the same image, as
  root, no network) that hands the folder to 10001 before kwnt starts. Umbrel creates app folders as
  uid 1000, so its listing runs kwnt as `1000:1000`. A fresh Render disk may belong to root, so
  `render.yaml` builds [`docker/render.Dockerfile`](../docker/render.Dockerfile): the released image
  plus a start step that hands `/data` to 10001 and then drops root with `setpriv` (nothing compiles).
- **The token comes from the store** wherever the store can make a secret (Umbrel, Coolify, Render),
  through `KWNT_API_TOKEN`. CasaOS has no secret generator, so kwnt makes its own in `/data/api-token`.
- **Pinned images.** CasaOS and Umbrel pin `tag@sha256:<index digest>` (multi-arch: amd64 and arm64);
  Coolify templates and Render use the tag, as their other entries do.

## A new release

`registry_bump.sh <version>` (in the release tools) bumps every listing here: the image tag and
index digest, CasaOS `version`/`update_at`, and Umbrel `version`. The upstream store copies (CasaOS
and Umbrel PRs from the alexsergivan forks) get the same edit as a new PR to each store.
