touch org.aoe4_overlay.desktop
desktop-file-edit \
--set-name="AOE4 Overlay" \
--set-comment="An overlay for Age of Empires IV" \
--set-icon="$(pwd)/src_images/icons/logo.png" \
--add-category="Game;" \
--set-key="Exec" --set-value="$(pwd)/target/release/aoe4_overlay" \
--set-key="Type" --set-value="Application" \
org.aoe4_overlay.desktop

chmod +x org.aoe4_overlay.desktop
cp org.aoe4_overlay.desktop ~/.local/share/applications/
chmod +x ~/.local/share/applications/org.aoe4_overlay.desktop
desktop-file-install --dir=~/.local/share/applications/ org.aoe4_overlay.desktop
update-desktop-database ~/.local/share/applications