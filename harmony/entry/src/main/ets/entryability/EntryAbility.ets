// The app's ability: the window it shows in, and the page (the XComponent the app draws into).
// The interface is the app's own (Rust, as the desktop's), so nothing of it is here.

import UIAbility from '@ohos.app.ability.UIAbility';
import AbilityConstant from '@ohos.app.ability.AbilityConstant';
import Want from '@ohos.app.ability.Want';
import window from '@ohos.window';
import hilog from '@ohos.hilog';
import { BusinessError } from '@ohos.base';
import procraft from 'libprocraft.so';

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
      // the callback gives an error to look at either way: nothing at all on some paths and an
      // object whose code is 0 on others (the tablet's own gives `{code: 0}` when it worked). So
      // it is read as one only where it has a non-zero code — reading `.code` off nothing is
      // itself a TypeError, and a TypeError nothing catches stops the process (it was the
      // "Kill Reason: Js Error" the app started with)
      if (error && error.code) {
        hilog.error(0x3200, TAG, 'the window: %{public}s', JSON.stringify(error));
        return;
      }
      // full screen, over the whole display (a floating window would leave the canvas in a
      // corner of it)
      main.setWindowLayoutFullScreen(true).catch((said: BusinessError) => {
        hilog.warn(0x3200, TAG, 'the full screen: %{public}s', JSON.stringify(said));
      });
      main.setWindowSystemBarEnable([]).catch((said: BusinessError) => {
        hilog.warn(0x3200, TAG, 'the system bars: %{public}s', JSON.stringify(said));
      });
      // the window fills the display (a floating one would leave the canvas in a corner of it)
      main.maximize(window.MaximizePresentation.ENTER_IMMERSIVE).catch((said: BusinessError) => {
        hilog.warn(0x3200, TAG, 'the window: %{public}s', JSON.stringify(said));
      });
      // the screen stays on while drawing
      main.setWindowKeepScreenOn(true).catch((said: BusinessError) => {
        hilog.warn(0x3200, TAG, 'the screen: %{public}s', JSON.stringify(said));
      });
      // wide colour, so a canvas's own colours show as they are
      main.setWindowColorSpace(window.ColorSpace.WIDE_GAMUT).catch((said: BusinessError) => {
        hilog.warn(0x3200, TAG, 'the colour: %{public}s', JSON.stringify(said));
      });
    });

    windowStage.loadContent('pages/Index', (error) => {
      if (error && error.code) {
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
