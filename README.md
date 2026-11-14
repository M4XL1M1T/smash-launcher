# Smash Launcher

A small, lightweight desktop launcher for Minecraft: Java Edition, written in Rust. It manages Microsoft accounts and starts any vanilla version, from old alpha builds to the latest release.

This is a personal, non-commercial hobby project. It is not affiliated with, endorsed by or sponsored by Mojang Studios or Microsoft.

[Deutsche Version](README.de.md)

## Authentication and game ownership

- **Online mode only.** There are no offline accounts and no option to skip the login.
- Sign-in uses the Microsoft device code flow (scope `XboxLive.signin offline_access`), then Xbox Live, XSTS and the Minecraft services login and profile endpoints.
- On **every launch** the launcher refreshes the sign-in and requests the Minecraft profile. If the account does not own Minecraft: Java Edition, the game is not started.
- Each user runs the launcher locally with their own account. There is no server component, no telemetry and no monetization.
- Tokens are stored locally in `data/config.json` next to the executable and are only sent to Microsoft, Xbox and Mojang endpoints. The file is currently not encrypted, so do not share it.

## Features

- Multiple accounts, switch with one click
- All release, snapshot, alpha and beta versions from Mojang's version manifest
- Automatic download of the client, libraries and assets, with SHA-1 verification
- Automatic download of the matching Java runtime from Mojang's runtime manifest (Java 8, 17, 21, ...), or a custom Java path in the settings
- One game directory per version in `data/instances/<version>`
- Local profiles that use `inheritsFrom` (for example Fabric) are picked up
- Small binary of about 4 MB, built with egui, no web runtime

## Usage

1. Download or build `smash-launcher.exe` and run it. All data is stored in a `data` folder next to it (set the `MCL_HOME` environment variable to use another folder).
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
- `src/main.rs`: user interface
