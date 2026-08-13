# Translating Ira

Extract the current source strings with:

```bash
po/update-pot.sh
```

Compile a locale catalog for development or packaging with:

```bash
msgfmt po/<locale>.po --output-file <locale>/LC_MESSAGES/ira.mo
```

Set `IRA_LOCALEDIR` to the parent directory containing locale folders when
running from an uninstalled build. Installed builds use `/usr/share/locale`.
