// The app's ability: the window it shows in, and the page (the XComponent the app draws into).
// The interface is the app's own (Rust, as the desktop's), so nothing of it is here.

import UIAbility from '@ohos.app.ability.UIAbility';
import AbilityConstant from '@ohos.app.ability.AbilityConstant';
import Want from '@ohos.app.ability.Want';
import window from '@ohos.window';
import hilog from '@ohos.hilog';

const TAG: string = 'ProCraft';

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the app starts');
  }

  onDestroy(): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the app ends');
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    // the canvas is the whole screen: no title bar, no system bars over it
    windowStage.getMainWindow((error, main) => {
      if (error.code) {
        hilog.error(0x3200, TAG, 'the window: %{public}s', JSON.stringify(error));
        return;
      }
      // full screen, over the whole display (a floating window would leave the canvas in a
      // corner of it)
      main.setWindowLayoutFullScreen(true);
      main.setWindowSystemBarEnable([]);
      // the window fills the display (a floating one would leave the canvas in a corner of it)
      main.maximize(window.MaximizePresentation.ENTER_IMMERSIVE);
      // the screen stays on while drawing
      main.setWindowKeepScreenOn(true);
      // wide colour, so a canvas's own colours show as they are
      main.setWindowColorSpace(window.ColorSpace.WIDE_GAMUT);
    });

    windowStage.loadContent('pages/Index', (error) => {
      if (error.code) {
        hilog.error(0x3200, TAG, 'the page: %{public}s', JSON.stringify(error));
        return;
      }
      hilog.info(0x3200, TAG, '%{public}s', 'the page is up');
    });
  }

  onWindowStageDestroy(): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the page is gone');
  }

  onForeground(): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the app is in front');
  }

  onBackground(): void {
    hilog.info(0x3200, TAG, '%{public}s', 'the app is behind');
  }
}
