---
name: deploy
description: Build and deploy the Life Manager app via Docker Compose
allowed-tools: Bash, Read, Glob
---

Deploy the Life Manager application.

Run: `./scripts/deploy.sh`

Verify: `docker compose logs app --tail 20` and `curl -s -o /dev/null -w "%{http_code}" https://lifemanager.tail6c1af7.ts.net/`

Report whether server functions registered and app returns 200. On failure, show error logs and suggest a fix.
