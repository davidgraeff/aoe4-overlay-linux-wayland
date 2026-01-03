# Gnome shell extension: Overlay on top

Keeps age of empires overlay tool always on top and moves the window
the the screen where the game is running.

There is no way to archive this programmatically in Gnome Wayland sessions at the moment. 

## Installation from source code
Run below in terminal one by one:
```sh
mkdir -p ~/.local/share/gnome-shell/extensions
cd ~/.local/share/gnome-shell/extensions
git clone "https://github.com/davidgraeff/aoe4overlay.git" "aoe4-overlay@davidgraeff.github.com"
cd aoe4-overlay@davidgraeff.github.com
glib-compile-schemas ./schemas/
```

After all is done: logout, login back (or reboot) and enable newly installed extension. Enjoy!

Develop with: `dbus-run-session gnome-shell --devkit --wayland`
See logs via: `journalctl -f -o cat /usr/bin/gnome-shell`
