# Smash Launcher

Lightweight Minecraft launcher in Rust (egui, ca. 4 MB). Verwaltet mehrere Accounts und startet beliebige Vanilla-Versionen (1.0 bis aktuell, inkl. Snapshots und Alpha/Beta). Lokale Fabric/Forge-Profile (`inheritsFrom`) werden ebenfalls erkannt.

## Starten

`smash-launcher.exe` ausführen. Beim ersten Start wählst du einen Ordner, in dem alles gespeichert wird (Config, Versionen, Assets, Java, Spielordner pro Version unter `instances\<version>`). Die Wahl wird in `smash-launcher.json` neben der exe gemerkt (Ausweich: `%APPDATA%\SmashLauncher\location.json`). Die Umgebungsvariable `MCL_HOME` überschreibt sie.

Java muss nicht installiert sein: der Launcher lädt automatisch die passende Mojang-Runtime (Java 8, 17, 21, ...) pro Version. Optional kann ein eigener Java-Pfad in den Einstellungen gesetzt werden.

## Modrinth-Import

Eine exportierte `.mrpack`-Datei (Fabric, Quilt oder Vanilla) lässt sich über den Button "Modrinth-Modpack importieren" oder per Drag and Drop ins Fenster importieren. Loader, Mods und Konfiguration landen in einem eigenen Profil. Forge und NeoForge werden noch nicht unterstützt.

## Download und Prüfung

Lade die Datei nur von der Seite **Releases** dieses Repositories. Sie ist noch nicht code-signiert, deshalb zeigt Windows SmartScreen beim ersten Start eventuell die Warnung "Unbekannter Herausgeber": auf **Weitere Informationen** und dann **Trotzdem ausführen** klicken. Jedes Release nennt die SHA-256-Prüfsumme der Datei. So prüfst du deinen Download in PowerShell:

```
Get-FileHash .\smash-launcher-vX.Y.Z-windows-x64.exe -Algorithm SHA256
```

Weicht der Wert von dem in den Release-Hinweisen ab, führe die Datei nicht aus. Du kannst den Launcher auch selbst bauen (siehe unten).

## Updates

Beim Start fragt der Launcher bei GitHub das neueste Release dieses Repositories ab. Gibt es eine neuere Version, erscheint oben ein Hinweis mit **Jetzt aktualisieren**: Der Launcher lädt die Datei aus dem Release, prüft sie gegen die `SHA256SUMS.txt` desselben Releases, ersetzt sich selbst und startet neu. Bei falscher Prüfsumme wird nichts installiert. Die Suche lässt sich in den Einstellungen (Tab Launcher) abschalten und dort auch manuell starten. Ein Release braucht dafür die Datei `smash-launcher-vX.Y.Z-windows-x64.exe`, die `SHA256SUMS.txt` und einen Tag `vX.Y.Z`, der höher ist als die installierte Version.

## Accounts

Nur Online-Modus: Es gibt bewusst keine Offline-Accounts. Bei jedem Start wird per Microsoft-Login geprüft, dass das Konto Minecraft Java besitzt. Jeder nutzt den Launcher lokal mit seinem eigenen Account, Tokens verlassen den PC nicht.

Login per Gerätecode. Dafür wird eine eigene Azure Client-ID benötigt (einmalig, kostenlos):
  1. https://portal.azure.com, App-Registrierung, neue Registrierung
  2. Kontotypen: "Persönliche Microsoft-Konten" (bzw. alle Konten inkl. persönlicher)
  3. Unter Authentifizierung "Öffentliche Clientflows zulassen" auf Ja
  4. Anwendungs-ID (Client-ID) im Launcher unter Einstellungen eintragen
  5. Hinweis: Neue Azure-Apps müssen für die Minecraft-API einmalig bei Mojang freigeschaltet werden (https://aka.ms/AppRegInfo), sonst kommt beim Login ein Fehler.

Tokens werden unverschlüsselt in `config.json` im Datenordner gespeichert. Datei nicht teilen.

## Bauen

```
cargo build --release
```
