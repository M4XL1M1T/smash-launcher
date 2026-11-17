# Smash Launcher

A small, lightweight desktop launcher for Minecraft: Java Edition, written in Rust. It manages Microsoft accounts and starts any vanilla version, from old alpha builds to the latest release.

This is a personal, non-commercial hobby project. It is not affiliated with, endorsed by or sponsored by Mojang Studios or Microsoft.

[Deutsche Version](README.de.md)

## Authentication and game ownership

- **Online mode only.** There are no offline accounts and no option to skip the login.
- Sign-in uses the Microsoft device code flow (scope `XboxLive.signin offline_access`), then Xbox Live, XSTS and the Minecraft services login and profile endpoints.
- On **every launch** the launcher refreshes the sign-in and requests the Minecraft profile. If the account does not own Minecraft: Java Edition, the game is not started.
- Each user runs the launcher locally with their own account. There is no server component, no telemetry and no monetization. The only other network request is the optional update check against the GitHub Releases API of this repository (see below).
- Tokens are stored locally in `config.json` inside the chosen data folder and are only sent to Microsoft, Xbox and Mojang endpoints. The file is currently not encrypted, so do not share it.

## Download and verification

Get the executable only from the [Releases](../../releases) page of this repository. It is not code signed yet, so Windows SmartScreen may show an "unknown publisher" warning the first time: click **More info**, then **Run anyway**. Every release lists the SHA-256 checksum of the file, you can verify your download in PowerShell:

```
Get-FileHash .\smash-launcher-vX.Y.Z-windows-x64.exe -Algorithm SHA256
```

If the hash does not match the one in the release notes, do not run the file. You can also build the launcher yourself, see below.

## Updates

On start the launcher asks GitHub for the latest release of this repository. If a newer version exists, a banner offers **Jetzt aktualisieren**: the launcher downloads the exe from the release, verifies it against `SHA256SUMS.txt` of the same release, replaces itself and restarts. Nothing is installed if the checksum does not match. The check can be turned off in the settings (tab Launcher), where you can also check manually. For this a release needs the asset `smash-launcher-vX.Y.Z-windows-x64.exe` and `SHA256SUMS.txt` (created by the release helper) and a tag `vX.Y.Z` that is higher than the installed version.

## Releasing (maintainer)

Releases are built by GitHub Actions (`.github/workflows/release.yml`), so the published exe comes from the source in this repository. One-time setup: add the repository secret `SMASH_CLIENT_ID` (Settings, Secrets and variables, Actions). To release: raise `version` in `Cargo.toml`, push, then run **Actions, Release, Run workflow** and enter the tag (for example `v0.2.1`). The workflow runs the tests, builds the exe and creates a **draft** release with the exe, `SHA256SUMS.txt` and release notes. Check the draft on the Releases page and click **Publish release**.

## Features

- Multiple accounts, switch with one click
- All release, snapshot, alpha and beta versions from Mojang's version manifest
- Automatic download of the client, libraries and assets, with SHA-1 verification
- Automatic download of the matching Java runtime from Mojang's runtime manifest (Java 8, 17, 21, ...), or a custom Java path in the settings
- One game directory per version in `data/instances/<version>`
- Local profiles that use `inheritsFrom` (for example Fabric) are picked up
- Import of Modrinth modpacks (`.mrpack`, Fabric, Quilt and vanilla packs): click the import button or drag the file into the window. The loader, all mods and the config overrides are installed into their own profile. Forge and NeoForge packs are not supported yet.
- Small binary of about 4 MB, built with egui, no web runtime

## Usage

1. Download or build `smash-launcher.exe` and run it. On the first start you choose a folder where everything is stored (versions, mods, saves, Java and account data). The choice is remembered in `smash-launcher.json` next to the executable (fallback: `%APPDATA%\SmashLauncher\location.json`). The `MCL_HOME` environment variable overrides it.
2. Click **Mit Microsoft anmelden**, open the shown link and enter the code.
3. Pick a version and click the play button. Missing files are downloaded first.

## Building

Requires a recent stable Rust toolchain.

```
cargo build --release
```

The Azure application (client) ID used for the sign-in is not stored in the repository. Set the `SMASH_CLIENT_ID` environment variable when building to embed it, or enter it in the launcher settings at runtime. To use your own app registration, create one in the Azure portal with the account type that includes personal Microsoft accounts and enable "Allow public client flows". Minecraft services only accept applications that Mojang has approved, see https://aka.ms/AppRegInfo.

## Project layout

- `src/auth.rs`: Microsoft, Xbox and Minecraft sign-in
- `src/game.rs`: version manifest, installation, Java runtime, command line and process handling
- `src/net.rs`: HTTP helpers and the parallel downloader
- `src/config.rs`: settings and accounts
- `src/mrpack.rs`: Modrinth modpack import
- `src/main.rs`: user interface
