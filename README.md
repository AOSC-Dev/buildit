# AOSC BuildIt! Bot

Build automation with Telegram and GitHub Integrations.

## Setting Up a BuildIt! Worker

Steps (as root):

0. Install git, ciel, pushpkg, rustc (with Cargo), compiler toolchain (LLVM/Clang)
1. `mkdir -p /buildroots/buildit`
2. `cd /buildroots/buildit && git clone https://github.com/AOSC-Dev/buildit`
3. `cd /buildroots/buildit && ciel new`, making sure to create an instance named "main" when asked
4. `cp /buildroots/buildit/buildit/systemd/buildit-worker.service /etc/systemd/system`
5. `$EDITOR /etc/systemd/system/buildit-worker.service`：update ARCH
6. `$EDITOR /buildroots/buildit/buildit/.env`: set BUILDIT_SERVER, BUILDIT_WORKER_SECRET and BUILDIT_SSH_KEY; for workers in China, optionally update BUILDIT_RSYNC_HOST to repo-cn.aosc.io
7. `systemctl enable --now buildit-worker`
8. `chmod 600 /buildroots/buildit/buildit/.env`
9. Setup SSH key of AOSC Maintainers at the location of BUILDIT_SSH_KEY
10. Add SSH known hosts from repo.aosc.io and github.com: `ssh-keyscan repo.aosc.io >> ~/.ssh/known_hosts && ssh-keyscan repo-cn.aosc.io >> ~/.ssh/known_hosts && ssh-keyscan github.com >> ~/.ssh/known_hosts`

Arch-specific notes:

- Add `--no-default-features --features gix-faster` to cargo for loongarch64 until libz-ng support lands
- Add `RUSTFLAGS="-C link-arg=-fuse-ld=gold"` environment for loongson3
