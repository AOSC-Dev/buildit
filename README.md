# AOSC BuildIt! Bot

Build automation with Telegram and GitHub Integrations.

## Setting Up a BuildIt! Worker

Steps (as root):

0. Install git, ciel, pushpkg, rustc (with Cargo), compiler toolchain (LLVM/Clang)
1. `mkdir -p /buildroots/buildit`
2. `cd /buildroots/buildit && git clone https://github.com/AOSC-Dev/buildit`
3. `cd /buildroots/buildit && ciel new` with maintainer called `AOSC OS Maintainers <maintainers@aosc.io>`, making sure to create an instance named "main" when asked
4. `cp /buildroots/buildit/buildit/systemd/buildit-worker.service /etc/systemd/system`
5. `$EDITOR /etc/systemd/system/buildit-worker.service`：update `ARCH`
6. `$EDITOR /buildroots/buildit/buildit/.env`: set `BUILDIT_SERVER`, `BUILDIT_WORKER_SECRET` `BUILDIT_SSH_KEY` and `BUILDIT_WORKER_PERFORMANCE`; for workers with special network environments, optionally set `BUILDIT_PUSHPKG_OPTIONS`
7. `systemctl enable --now buildit-worker`
8. `chmod 600 /buildroots/buildit/buildit/.env`
9. Generate a new SSH key at the location of `BUILDIT_SSH_KEY`, and setup `authorized_keys` on repo.aosc.io (contact infra team)
10. Add SSH known hosts from repo.aosc.io and github.com: `ssh-keyscan repo.aosc.io >> ~/.ssh/known_hosts && ssh-keyscan github.com >> ~/.ssh/known_hosts`

Arch-specific notes:

- Add `RUSTFLAGS="-C link-arg=-fuse-ld=gold"` environment for loongson3
