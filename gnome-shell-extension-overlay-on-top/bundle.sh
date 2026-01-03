UUID="aoe4-overlay@davidgraeff.github.com"
ZIPFILES="extension.js prefs.js metadata.json schemas"

glib-compile-schemas ./schemas/
zip -qr "$UUID.zip" $ZIPFILES
