/*
 * GNOME Shell Extension: PiP on top
 * Developer: Rafostar
 */

import Meta from 'gi://Meta';
import {Extension, gettext as _} from 'resource:///org/gnome/shell/extensions/extension.js';

export default class PipOnTop extends Extension
{
  enable()
  {
    console.log(`Age of Empires 4 Overlay always on top v4`);
    this._lastWorkspace = null;
    this._windowAddedId = 0;
    this._windowRemovedId = 0;
    this._gameWindow = null;
    this._overlayWindow = null;

    this.settings = this.getSettings();
    this._settingsChangedId = this.settings.connect(
      'changed', this._onSettingsChanged.bind(this));

    this._switchWorkspaceId = global.window_manager.connect_after(
      'switch-workspace', this._onSwitchWorkspace.bind(this));
    this._onSwitchWorkspace();
  }

  disable()
  {
    this.settings.disconnect(this._settingsChangedId);
    this.settings = null;

    global.window_manager.disconnect(this._switchWorkspaceId);

    if (this._lastWorkspace) {
      this._lastWorkspace.disconnect(this._windowAddedId);
      this._lastWorkspace.disconnect(this._windowRemovedId);
    }

    this._lastWorkspace = null;
    this._settingsChangedId = 0;
    this._switchWorkspaceId = 0;
    this._windowAddedId = 0;
    this._windowRemovedId = 0;
    this._gameWindow = null;
    this._overlayWindow = null;

    let actors = global.get_window_actors();
    if (actors) {
      for (let actor of actors) {
        let window = actor.meta_window;
        if (!window) continue;

        if (window._isPipAble) {
          if (window.above)
            window.unmake_above();
          if (window.on_all_workspaces)
            window.unstick();
        }

        this._onWindowRemoved(null, window);
      }
    }
  }

  _onSettingsChanged(settings, key)
  {
    switch (key) {
      case 'stick':
      case 'overlay-window-title':
      case 'game-window-title':
        /* Updates already present windows */
        this._onSwitchWorkspace();
        break;
      default:
        break;
    }
  }

  _onSwitchWorkspace()
  {
    let workspace = global.workspace_manager.get_active_workspace();
    let wsWindows = global.display.get_tab_list(Meta.TabList.NORMAL, workspace);

    if (this._lastWorkspace) {
      this._lastWorkspace.disconnect(this._windowAddedId);
      this._lastWorkspace.disconnect(this._windowRemovedId);
    }

    this._lastWorkspace = workspace;
    this._windowAddedId = this._lastWorkspace.connect(
      'window-added', this._onWindowAdded.bind(this));
    this._windowRemovedId = this._lastWorkspace.connect(
      'window-removed', this._onWindowRemoved.bind(this));

    this._clearLoggedTitles();

    /* Update state on already present windows */
    if (wsWindows) {
      for (let window of wsWindows)
        this._onWindowAdded(workspace, window, true);
    }
  }

  _onWindowAdded(workspace, window, is_static = false)
  {
    //console.log(`Add window: ${window.title} (existing: ${is_static})`);
    // Log that a window was added
    if (!window._notifyPipTitleId) {
      window._notifyPipTitleId = window.connect_after(
        'notify::title', this._checkTitle.bind(this));
    }
    this._checkTitle(window);
  }

  _onWindowRemoved(workspace, window)
  {
    if (window._notifyPipTitleId) {
      window.disconnect(window._notifyPipTitleId);
      window._notifyPipTitleId = null;
    }
    if (window._notifyPipPositionId) {
      window.disconnect(window._notifyPipPositionId);
      window._notifyPipPositionId = null;
    }
    if (window._isPipAble)
      window._isPipAble = null;
    
    /* Clear references if windows are closed */
    if (window === this._gameWindow)
      this._gameWindow = null;
    if (window === this._overlayWindow)
      this._overlayWindow = null;
  }

  _clearLoggedTitles()
  {
    this.settings.set_strv('detected-window-titles', []);
  }

  _logWindowTitle(window)
  {
    if (!window.title)
      return;
    
    /* Log window title for debugging */
    let detectedTitles = this.settings.get_strv('detected-window-titles');
    
    /* Add new title if not already in list */
    if (!detectedTitles.includes(window.title)) {
      detectedTitles.unshift(window.title);
      /* Keep only last 50 titles */
      if (detectedTitles.length > 50)
        detectedTitles = detectedTitles.slice(0, 50);
      
      this.settings.set_strv('detected-window-titles', detectedTitles);
    }
  }

  _checkTitle(window)
  {
    if (!window.title)
      return;

    this._logWindowTitle(window);

    /* Check for the configured overlay window */
    let overlayTitle = this.settings.get_string('overlay-window-title');
    let gameTitle = this.settings.get_string('game-window-title');
    let isDesiredWindow = (window.title == overlayTitle);

    /* Check for the Age of Empires IV game window */
    let isGameWin = window.title.startsWith(gameTitle);

    if (isDesiredWindow || window._isPipAble) {
      let un = (isDesiredWindow) ? '' : 'un';

      console.log(`Found window: ${window.title}`);

      window._isPipAble = true;
      window[`${un}make_above`]();

      /* Change stick if enabled or unstick PipAble windows */
      un = (isDesiredWindow && this.settings.get_boolean('stick')) ? '' : 'un';
      window[`${un}stick`]();
      
      /* Track overlay window */
      if (isDesiredWindow) {
        this._overlayWindow = window;
        this._moveOverlayToGameMonitor();
      }
    }
    
    /* Track game window and monitor its position */
    if (isGameWin) {
      this._gameWindow = window;
      
      /* Monitor when game window moves to different monitor */
      if (!window._notifyPipPositionId) {
        window._notifyPipPositionId = window.connect(
          'position-changed', this._onGamePositionChanged.bind(this));
      }
      
      /* Move overlay to game's monitor initially */
      this._moveOverlayToGameMonitor();
    }
  }
  
  _onGamePositionChanged(window)
  {
    /* When game window moves, update overlay position */
    this._moveOverlayToGameMonitor();
  }
  
  _moveOverlayToGameMonitor()
  {
    console.log(`_moveOverlayToGameMonitor: ${this._gameWindow?.title} | ${this._overlayWindow?.title}`);

    if (!this._gameWindow || !this._overlayWindow)
      return;
    
    /* Get the monitor index where the game window is located */
    let gameMonitorIndex = this._gameWindow.get_monitor();
    let overlayMonitorIndex = this._overlayWindow.get_monitor();
    
    console.log(`Game monitor: ${gameMonitorIndex}, Overlay monitor: ${overlayMonitorIndex}`);
    /* Only move if they're on different monitors */
    if (gameMonitorIndex !== overlayMonitorIndex) {
      let monitor = global.display.get_monitor_geometry(gameMonitorIndex);
      
      /* Move overlay window to the game's monitor */
      this._overlayWindow.move_frame(false, monitor.x, monitor.y);
    }
  }
}
