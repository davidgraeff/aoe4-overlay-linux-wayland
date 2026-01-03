import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';

import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

function _addToggle(group, settings, title, key)
{
  const toggleRow = new Adw.SwitchRow({
    title: title,
    active: settings.get_boolean(key),
  });
  settings.bind(key, toggleRow, 'active',
    Gio.SettingsBindFlags.DEFAULT);
  group.add(toggleRow);
}

function _addEntry(group, settings, title, subtitle, key)
{
  const entryRow = new Adw.EntryRow({
    title: title,
    text: settings.get_string(key),
  });
  
  if (subtitle)
    entryRow.subtitle = subtitle;
  
  entryRow.connect('changed', () => {
    settings.set_string(key, entryRow.text);
  });
  
  group.add(entryRow);
}

export default class PreferencesWindow extends ExtensionPreferences {
    _onSettingsChanged(settings, key)
    {
      switch (key) {
        case 'detected-window-titles':
          let detectedTitles = this.settings.get_strv('detected-window-titles');
          console.log(`Updated detected window titles: ${detectedTitles}`);
          this._clearRows();
          this._updateRows();
          break;
        default:
          break;
      }
    }
    _clearRows() {
      // Remove all rows from expander
      let child = this._expanderRow.get_first_child();
      while (child) {
        const next = child.get_next_sibling();
        if (child !== this._expanderRow.get_first_child()) {
          this._expanderRow.remove(child);
        }
        child = next;
      }
    }
    _updateRows() {
      const detectedTitles = this.settings.get_strv('detected-window-titles');
      
      if (detectedTitles.length === 0) {
        const emptyRow = new Adw.ActionRow({
          title: 'No windows detected yet',
        });
        this._expanderRow.add_row(emptyRow);
      } else {
        detectedTitles.forEach((title, index) => {
          const row = new Adw.ActionRow({
            title: title,
          });
          this._expanderRow.add_row(row);
        });
      }
    }

    fillPreferencesWindow(window)
    {
      const settings = this.getSettings();
      this.settings = settings;
      this._settingsChangedId = this.settings.connect(
      'changed', this._onSettingsChanged.bind(this));

      const page = new Adw.PreferencesPage();
      
      // Configuration group
      const configGroup = new Adw.PreferencesGroup({
        title: 'Configuration',
      });

      _addEntry(configGroup, settings, 'Overlay Window Title', 
        'The exact window title to keep always on top', 'overlay-window-title');
      _addEntry(configGroup, settings, 'Game Window Title', 
        'The exact window title of the game window', 'game-window-title');
      _addToggle(configGroup, settings, 'Show on all workspaces', 'stick');

      page.add(configGroup);
      
      // Detected windows group
      const debugGroup = new Adw.PreferencesGroup({
        title: 'Detected Window Titles',
        description: 'Recently detected window titles (for debugging)',
      });
      
      // Create expander row for window titles list
      const expanderRow = new Adw.ExpanderRow({
        title: 'Show detected windows',
      });
      this._expanderRow = expanderRow;
      this._clearRows();
      this._updateRows();
      
      debugGroup.add(expanderRow);
      
      page.add(debugGroup);
      window.add(page);
    }
}

