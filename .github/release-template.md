# Smash Launcher v{{VERSION}}

A small, lightweight launcher for Minecraft: Java Edition with Microsoft accounts (online mode only), Modrinth modpack import (.mrpack with Fabric or Quilt) and automatic Java download. Windows x64.

Not affiliated with, endorsed by or sponsored by Mojang Studios or Microsoft. You need to own Minecraft: Java Edition. The ownership is checked on every launch.

## Download

- `{{NAME}}`
- `SHA256SUMS.txt`

If you already use Smash Launcher, it offers this version as an update on start.

## Windows SmartScreen warning

The executable is not code signed yet, so Windows may show "Windows protected your PC" with an unknown publisher the first time you start it. If you downloaded it from this release page, click **More info**, then **Run anyway**. To make sure the file is the one published here, compare its SHA-256 checksum before running it.

Expected SHA-256:

```
{{HASH}}
```

Check it in PowerShell (run it in the folder with the download):

```
Get-FileHash .\{{NAME}} -Algorithm SHA256
```

The printed hash must be identical to the one above and to the one in `SHA256SUMS.txt`. If it differs, do not run the file.

## Privacy

Tokens are stored only on your own computer in the data folder you choose on the first start (not encrypted, do not share it). There is no telemetry and no server component. The only extra request is the optional update check against the GitHub Releases API, which can be turned off in the settings. The exe of this release was built by GitHub Actions from the source code in this repository.
