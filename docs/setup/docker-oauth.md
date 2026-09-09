# Docker OAuth and persistent profiles

Use an installed Temm1e binary on a machine with a browser to authenticate. Export into a **private profile directory**, then mount that whole writable directory. OAuth refresh replaces the token file atomically and uses a sibling lock and recovery marker; a single-file bind mount cannot support that replacement.

```bash
mkdir -m 700 ./temm1e-profile
temm1e auth login --output ./temm1e-profile/oauth.json
```

The export is private (0600 on Unix). Login also saves the local profile's credentials. Treat both copies as secrets. For a remote host, transfer the directory through your usual secure channel and preserve its permissions. Run only one deployment against a given rotating refresh token; independently refreshing copies can invalidate one another.

```yaml
services:
  temm1e:
    image: temm1e:latest
    environment:
      TEMM1E_DATA_DIR: /var/lib/temm1e
      TELEGRAM_BOT_TOKEN: ${TELEGRAM_BOT_TOKEN}
    volumes:
      - ./temm1e-profile:/var/lib/temm1e
    restart: unless-stopped
```

```bash
docker build -t temm1e:latest .
docker compose up -d
```

The directory persists OAuth state, configuration, conversations, memory and other profile data. Place optional configuration at `temm1e-profile/config.toml`. Ensure the container user can write this directory. Do not commit it to Git or place it in a public/shared folder.

## Refresh and reconnect

Refresh timing comes from the provider's token metadata; a fixed refresh-token lifetime is not guaranteed. Temm1e locks the token store before refresh and writes a pending marker before the request. An interrupted or ambiguous refresh can require reconnecting instead of retrying a potentially consumed refresh token.

To reconnect, stop the container, repeat the login/export into its mounted profile, and start the container again. Do not overwrite credentials while a refresh is running.

For a headless machine:

```bash
temm1e auth login --headless --output ./temm1e-profile/oauth.json
```

Open the printed URL in a browser and follow the terminal instructions to return the redirect URL.

## Kubernetes

Mount a writable persistent volume at `/var/lib/temm1e` and set `TEMM1E_DATA_DIR` to that path. A Kubernetes Secret projection is read-only: do not mount it directly as the live `oauth.json`, including through `subPath`.

If bootstrapping from a Secret, an init container may copy it into the private persistent directory **only when the live token does not exist**. Preserve refreshed credentials on subsequent starts. Coordinate explicit reconnects separately; never overwrite the live file from the bootstrap Secret on every restart. Use one replica per rotating token/profile unless a separately validated shared ownership mechanism exists.

## Existing containers

Before replacing an older image or changing mounts, follow the [profile migration instructions](../ops/deployment.md#existing-container-migration). Older images did not honor their advertised `TEMM1E_HOME=/data`; your active profile may be inside `/root/.temm1e` in the container's writable layer.

| Symptom | Check |
| --- | --- |
| No OAuth tokens found | `oauth.json` exists in the directory selected by `TEMM1E_DATA_DIR` |
| Permission denied or resource busy during refresh | Whole directory is writable; token is not a file bind mount/read-only Secret |
| Reconnect required | Complete a new login while the deployment is stopped |
| State disappears after container recreation | Mount the actual profile directory, not an unused path |
