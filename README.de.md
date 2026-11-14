# Smash Launcher

Lightweight Minecraft launcher in Rust (egui, ca. 4 MB). Verwaltet mehrere Accounts und startet beliebige Vanilla-Versionen (1.0 bis aktuell, inkl. Snapshots und Alpha/Beta). Lokale Fabric/Forge-Profile (`inheritsFrom`) werden ebenfalls erkannt.

## Starten

`dist\smash-launcher.exe` ausführen. Alle Daten liegen portabel in `dist\data\` (Config, Versionen, Assets, Java, Spielordner pro Version unter `instances\<version>`). Mit der Umgebungsvariable `MCL_HOME` lässt sich ein anderer Datenordner setzen.

Java muss nicht installiert sein: der Launcher lädt automatisch die passende Mojang-Runtime (Java 8, 17, 21, ...) pro Version. Optional kann ein eigener Java-Pfad in den Einstellungen gesetzt werden.

## Accounts

Nur Online-Modus: Es gibt bewusst keine Offline-Accounts. Bei jedem Start wird per Microsoft-Login geprüft, dass das Konto Minecraft Java besitzt. Jeder nutzt den Launcher lokal mit seinem eigenen Account, Tokens verlassen den PC nicht.

Login per Gerätecode. Dafür wird eine eigene Azure Client-ID benötigt (einmalig, kostenlos):
  1. https://portal.azure.com, App-Registrierung, neue Registrierung
  2. Kontotypen: "Persönliche Microsoft-Konten" (bzw. alle Konten inkl. persönlicher)
  3. Unter Authentifizierung "Öffentliche Clientflows zulassen" auf Ja
  4. Anwendungs-ID (Client-ID) im Launcher unter Einstellungen eintragen
  5. Hinweis: Neue Azure-Apps müssen für die Minecraft-API einmalig bei Mojang freigeschaltet werden (https://aka.ms/AppRegInfo), sonst kommt beim Login ein Fehler.

Tokens werden unverschlüsselt in `data\config.json` gespeichert. Datei nicht teilen.

## Bauen

```
cargo build --release
```
