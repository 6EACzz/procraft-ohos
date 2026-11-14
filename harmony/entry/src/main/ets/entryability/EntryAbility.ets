// The app's ability: the window it shows in, and the page (the XComponent the app draws into).
// The interface is the app's own (Rust, as the desktop's), so nothing of it is here.

import UIAbility from '@ohos.app.ability.UIAbility';
import AbilityConstant from '@ohos.app.ability.AbilityConstant';
import Want from '@ohos.app.ability.Want';
import window from '@ohos.window';
import ConfigurationConstant from '@ohos.app.ability.ConfigurationConstant';
import EnvironmentCallback from '@ohos.app.ability.EnvironmentCallback';
import { Configuration as AbilityConfiguration } from '@ohos.app.ability.Configuration';
import hilog from '@ohos.hilog';
import { BusinessError } from '@ohos.base';
import procraft from 'libprocraft.so';

const TAG: string = 'ProCraft';
// The system bars, by the names the window API takes: the status bar down the side of a tablet and
// the navigation bar along its bottom are the two the app has to hand back when its window stops
// filling the display.
const SYSTEM_BARS: Array<'status' | 'navigation'> = ['status', 'navigation'];

export default class EntryAbility extends UIAbility {
  // The window the app shows in, kept so that a colour-mode change can reach it (the page is the
  // app's own drawing and does not change with the mode; the window's own title bar does).
  private main: window.Window | undefined = undefined;
  // The mode last handed to the window. `setColorMode` is itself a change of configuration, so the
  // listener below hears about the very change it asked for: without this it would ask again, for
  // ever, at every tick of the loop (which is what it did).
  private told: ConfigurationConstant.ColorMode = ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET;

  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the app starts');
    // The system's light or dark mode changes are listened for here rather than by overriding
    // `onConfigurationUpdate`: that method's parameter is the interface's own `Configuration`
    // (whose `colorMode` is a word), which this file cannot pass on without knowing both shapes,
    // while this listener's parameter is the ability's, whose `colorMode` is the enum the window
    // takes. The two `Configuration` interfaces are named apart above for that reason.
    const listener: EnvironmentCallback = {
      onConfigurationUpdated: (config: AbilityConfiguration): void => {
        hilog.info(0x3200, TAG, 'the mode is now %{public}d', config.colorMode as number);
        if (this.main !== undefined) {
          this.applyColorMode(this.main, config.colorMode);
        }
      },
      onMemoryLevel: (level: AbilityConstant.MemoryLevel): void => {
        hilog.info(0x3200, TAG, 'the memory is now %{public}d', level as number);
      }
    };
    this.context.getApplicationContext().on('environment', listener);
  }

  onDestroy(): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the app ends');
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    // the canvas is the whole screen: no title bar, no system bars over it
    windowStage.getMainWindow((error, main) => {
      // the callback gives an error to look at either way: nothing at all on some paths and an
      // object whose code is 0 on others (the tablet's own gives `{code: 0}` when it worked). So
      // it is read as one only where it has a non-zero code — reading `.code` off nothing is
      // itself a TypeError, and a TypeError nothing catches stops the process (it was the
      // "Kill Reason: Js Error" the app started with)
      if (error && error.code) {
        hilog.error(0x3200, TAG, 'the window: %{public}s', JSON.stringify(error));
        return;
      }
      // How the app arrives: the canvas wants the whole display, so that is what it asks for. It is
      // a request — the system decides, and the tablet's own gestures can float the window or split
      // it at any time afterwards, which is what the listener below is for.
      main.maximize(window.MaximizePresentation.ENTER_IMMERSIVE).catch((said: BusinessError) => {
        hilog.warn(0x3200, TAG, 'the window: %{public}s', JSON.stringify(said));
      });
      // The window is laid out for the mode it is in, and told again whenever that changes: the
      // canvas is the whole display in full screen, and a window the system has put in a corner of
      // the display keeps its own frame (see `followWindowMode`). It is told again on every change
      // because the tablet's own gesture for a floating window must not be undone by what was set
      // up for the full screen the app started in.
      this.followWindowMode(main);
      main.on('windowStatusChange', (status: window.WindowStatusType) => {
        this.followWindowMode(main);
      });
      // the screen stays on while drawing
      main.setWindowKeepScreenOn(true).catch((said: BusinessError) => {
        hilog.warn(0x3200, TAG, 'the screen: %{public}s', JSON.stringify(said));
      });
      // wide colour, so a canvas's own colours show as they are
      main.setWindowColorSpace(window.ColorSpace.WIDE_GAMUT).catch((said: BusinessError) => {
        hilog.warn(0x3200, TAG, 'the colour: %{public}s', JSON.stringify(said));
      });
      this.main = main;
    });

    windowStage.loadContent('pages/Index', (error) => {
      if (error && error.code) {
        hilog.error(0x3200, TAG, 'the page: %{public}s', JSON.stringify(error));
        return;
      }
      hilog.info(0x3200, TAG, '%{public}s', 'the page is up');
      // the window's title bar is drawn by the system, not by the app, so it is the one part of
      // the window that has to be told which mode the system is in: without it the buttons stay
      // light while the system is dark, which is what showed in a floating window. It is told
      // once the page is up because a window that has not shown yet answers 1300002 (its state
      // is not one it will take a change in), which is what the first attempt here got.
      if (this.main !== undefined) {
        this.applyColorMode(this.main, this.context.config.colorMode);
      }
    });
  }

  // Lays the window out for the mode it is showing in. The system, not the app, decides which one
  // that is — a gesture on the tablet can float the window at any time — so what the app does here
  // is follow: a window that fills the display is drawn edge to edge with no bars over it, and one
  // that the system has given a frame of its own is drawn inside that frame instead, with its
  // title bar back. The screen may be laid out under the system bars or not, and that choice is the
  // only one either way.
  private followWindowMode(main: window.Window): void {
    let status: window.WindowStatusType = window.WindowStatusType.UNDEFINED;
    try {
      status = main.getWindowStatus();
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the window mode: %{public}s', JSON.stringify(error));
      return;
    }
    const framed: boolean =
      status === window.WindowStatusType.FLOATING || status === window.WindowStatusType.SPLIT_SCREEN;
    hilog.info(0x3200, TAG, 'the window is shown as %{public}d', status as number);
    // a window with a frame of its own keeps its title bar: drawing under it would put the top of
    // the canvas behind the system's own buttons
    main.setWindowLayoutFullScreen(!framed).catch((said: BusinessError) => {
      hilog.warn(0x3200, TAG, 'the full screen: %{public}s', JSON.stringify(said));
    });
    // and the bars come back with it, for the same reason
    main.setWindowSystemBarEnable(framed ? SYSTEM_BARS : []).catch((said: BusinessError) => {
      hilog.warn(0x3200, TAG, 'the system bars: %{public}s', JSON.stringify(said));
    });
  }

  // Tells the app, and with it the window's own decoration, which colour mode to draw in. The
  // decoration is the system's drawing rather than the app's, so it is the one part of the window
  // that cannot follow the canvas: left alone it keeps the light bar it starts with, which is what
  // showed in a floating window on a tablet whose system was dark.
  private applyColorMode(main: window.Window, mode: ConfigurationConstant.ColorMode | undefined): void {
    if (mode === undefined || mode === ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET) {
      return;
    }
    // nothing to do when the window is already in that mode — and everything to lose by asking
    // again: the ask is a configuration change, so the listener hears it and calls this back
    if (mode === this.told) {
      return;
    }
    this.told = mode;
    // the app's own resolution of its resources, which the decoration is drawn from: this is what
    // turns the bar dark, and it is why it is done before the buttons are told anything
    try {
      this.context.getApplicationContext().setColorMode(mode);
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the mode of the app: %{public}s', JSON.stringify(error));
    }
    // and the buttons on that bar, which the mode above does not reach
    try {
      main.setDecorButtonStyle({ colorMode: mode });
      hilog.info(0x3200, TAG, 'the title bar follows the mode: %{public}d', mode);
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the title bar: %{public}s', JSON.stringify(error));
    }
  }

  onWindowStageDestroy(): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the page is gone');
  }

  onForeground(): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the app is in front');
    // drawn again as it was: a touch already down when the app went behind is not still down
    // when it comes back, so the app is told it may take input again
    procraft.focused(true);
  }

  onBackground(): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the app is behind');
    // nothing is drawn behind other apps, and nothing is touched: the app lets go of whatever
    // it had in hand (a finger down would otherwise stay down forever)
    procraft.focused(false);
  }

  // The page is going away with the app: its surface is destroyed by the page itself (see
  // `OnSurfaceDestroyed`), so nothing of the app's own has to be torn down here.
}
