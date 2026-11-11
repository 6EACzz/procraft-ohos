// ProCraft on the tablet: the page the app shows. The interface itself is the desktop's own,
// drawn by the app's Rust code into the XComponent's surface (see `procraft/desktop/src/
// ohos_main.rs`); this page is what the system talks to — the surface, the file picker, the
// clipboard, the soft keyboard — and passes it on.

import { fileIo as fs } from '@kit.CoreFileKit';
import { picker } from '@kit.CoreFileKit';
import { pasteboard } from '@kit.BasicServicesKit';
import { image } from '@kit.ImageKit';
import { promptAction } from '@kit.ArkUI';
import { common } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import hilog from '@ohos.hilog';
import procraft from 'libprocraft.so';

const TAG: string = 'ProCraft';

// One ask the app made, as `asks()` hands it over.
interface Ask {
  id: number;
  kind: string;
  // The app writes its details as JSON inside the one JSON it hands over, so they come out of
  // that parse as a value already, not as words: what it is, a `PickDetails` or a `SayDetails`,
  // goes by the kind.
  details: Object;
}

// What a Pick's details say.
// One kind of file the picker offers.
interface FileFilter {
  name: string;
  endings: Array<string>;
}

interface PickDetails {
  title: string;
  name: string | null;
  filters: Array<FileFilter>;
  save: boolean;
  folder: boolean;
  several: boolean;
}

// What a clipboard write's details say.
interface WriteTextDetails {
  text: string;
}

// Whether the soft keyboard is shown.
interface KeyboardDetails {
  shown: boolean;
}

// What a Say's details say.
interface SayDetails {
  title: string;
  text: string;
  error: boolean;
}

@Entry
@Component
struct Index {
  // The app's own surface.
  private surface: XComponentController = new XComponentController();
  // Whether the app's own code is drawing (its surface came up).
  @State private ready: boolean = false;
  // The soft keyboard, shown while a text field of the app's has the focus.
  @State private typing: boolean = false;

  aboutToAppear(): void {
    // the app's own files go in this app's sandbox, which only this side can name: told as early
    // as it can be (a save may come before the surface does)
    this.tellOwnFolder();
    // the app's asks are taken as they come (it runs on its own thread, drawing by itself)
    this.pumpTimer = setInterval(() => this.pump(), 16);
  }

  // Where the app keeps its own files: this app's sandbox, which nothing under $HOME is.
  private tellOwnFolder(): void {
    try {
      const dir = this.context().filesDir;
      if (dir !== undefined && dir.length > 0) {
        procraft.setOwnFolder(dir);
        hilog.info(0x3200, TAG, 'the app keeps its files in %{public}s', dir);
      }
    } catch (error) {
      hilog.error(0x3200, TAG, 'the own folder: %{public}s', JSON.stringify(error));
    }
  }

  aboutToDisappear(): void {
    if (this.pumpTimer >= 0) {
      clearInterval(this.pumpTimer);
      this.pumpTimer = -1;
    }
    procraft.focused(false);
  }

  // Answers one of the app's asks. Each returns at once: a file picked is answered when the
  // system's picker hands it back (see `pick`).
  // The ability's own context, for the pickers.
  private context(): common.Context {
    return this.getUIContext().getHostContext() as common.Context;
  }

  // A thrown thing as words the log can show: a `BusinessError` has a code and a message, and
  // one that has neither reads as "{}" (which says nothing to act on).
  private why(error: Object): string {
    if (error === undefined || error === null) {
      return String(error);
    }
    const said = error as BusinessError;
    if (said.code !== undefined || said.message !== undefined) {
      return `${said.code}: ${said.message}`;
    }
    return error.toString();
  }

  private answer(ask: Ask): void {
    // an ask the app made that can't be read must not take the app down with it: the system's
    // own pickers throw, and an exception nothing catches stops the process (a Js Error)
    try {
      this.answerOne(ask);
    } catch (error) {
      hilog.error(0x3200, TAG, 'the ask %{public}s couldn\'t be answered: %{public}s',
        ask.kind, this.why(error));
      try {
        procraft.answer(ask.id, '', '', false);
      } catch (again) {
        // (the app is going anyway)
      }
    }
  }

  private answerOne(ask: Ask): void {
    const kind = ask.kind;
    const details = ask.details;
    hilog.info(0x3200, TAG, 'the app asked: %{public}s', kind);
    switch (kind) {
      case 'pick':
        this.pendingPick = ask.id;
        this.pick(details as PickDetails);
        break;
      case 'say':
        this.say(details as SayDetails);
        procraft.answer(ask.id, '', '', true);
        break;
      case 'clipboard-read':
        this.pendingClipboard = ask.id;
        this.readClipboard();
        break;
      case 'clipboard-write-text':
        this.writeClipboardText((details as WriteTextDetails).text);
        procraft.answer(ask.id, '', '', true);
        break;
      case 'clipboard-write-picture':
        procraft.answer(ask.id, '', '', true);
        break;
      case 'keyboard':
        this.typing = (details as KeyboardDetails).shown;
        if (this.typing) {
          this.showKeyboard();
        } else {
          this.hideKeyboard();
        }
        procraft.answer(ask.id, '', '', true);
        break;
      default:
        procraft.answer(ask.id, '', '', true);
        break;
    }
  }

  // A file or folder the app asked for: the system's own picker, and its answer once picked.
  private async pick(ask: PickDetails): Promise<void> {
    try {
      if (ask.folder) {
        const options = new picker.DocumentSelectOptions();
        options.maxSelectNumber = 1;
        const chosen = new picker.DocumentViewPicker(this.context());
        const picked = await chosen.select(options);
        this.answerPaths(picked);
        return;
      }
      if (ask.save) {
        const options = new picker.DocumentSaveOptions();
        options.newFileNames = [ask.name ?? 'untitled'];
        const chosen = new picker.DocumentViewPicker(this.context());
        const saved = await chosen.save(options);
        this.answerPaths(saved);
        return;
      }
      const options = new picker.DocumentSelectOptions();
      options.maxSelectNumber = ask.several ? 20 : 1;
      const kinds: Array<string> = [];
      for (const filter of ask.filters) {
        for (const ending of filter.endings) {
          kinds.push('.' + ending.toLowerCase());
        }
      }
      if (kinds.length > 0) {
        options.fileSuffixFilters = kinds;
      }
      const chosen = new picker.DocumentViewPicker(this.context());
      const picked = await chosen.select(options);
      this.answerPaths(picked);
    } catch (error) {
      const failed = error as BusinessError;
      hilog.warn(0x3200, TAG, 'the picker: %{public}s', this.why(error));
      this.answerPaths([]);
    }
  }

  // The paths picked, as the app reads them (each on its own line); none, cancelled.
  private answerPaths(paths: Array<string>): void {
    const text = paths.join('\n');
    const at = this.pendingPick;
    this.pendingPick = -1;
    if (at >= 0) {
      procraft.answer(at, text, '', true);
    }
  }

  private say(details: SayDetails): void {
    promptAction.showDialog({
      title: details.title,
      message: details.text,
      buttons: [{ text: 'OK', color: '#4F46E5' }],
    }).catch((error: BusinessError) => {
      hilog.warn(0x3200, TAG, 'the message: %{public}s', JSON.stringify(error));
    });
  }

  // What the system's clipboard holds, as the app reads it.
  private async readClipboard(): Promise<void> {
    try {
      const board = pasteboard.getSystemPasteboard();
      const data = await board.getData();
      // a picture first, then files, then words
      if (data.hasType(pasteboard.MIMETYPE_PIXELMAP)) {
        const pixels = data.getPrimaryPixelMap();
        if (pixels !== undefined) {
          const info: image.ImageInfo = await pixels.getImageInfo();
          const size: image.Size = info.size;
          const buffer = new ArrayBuffer(size.width * size.height * 4);
          await pixels.readPixelsToBuffer(buffer);
          const bytes = new Uint8Array(buffer);
          let hex = '';
          for (let at = 0; at < bytes.length; at++) {
            hex += bytes[at].toString(16).padStart(2, '0');
          }
          this.answerClipboard(`picture:${size.width}:${size.height}:${hex}`);
          return;
        }
      }
      if (data.hasType(pasteboard.MIMETYPE_TEXT_URI)) {
        const uri = data.getPrimaryText();
        if (uri !== undefined && uri.length > 0) {
          this.answerClipboard(`files:${uri}`);
          return;
        }
      }
      const text = data.getPrimaryText();
      this.answerClipboard(text !== undefined && text.length > 0 ? `text:${text}` : 'nothing');
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the clipboard: %{public}s', JSON.stringify(error));
      this.answerClipboard('nothing');
    }
  }

  private answerClipboard(clipboard: string): void {
    const at = this.pendingClipboard;
    this.pendingClipboard = -1;
    if (at >= 0) {
      procraft.answer(at, '', clipboard, true);
    }
  }

  private writeClipboardText(text: string): void {
    const data = pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, text);
    pasteboard.getSystemPasteboard().setData(data).catch((error: BusinessError) => {
      hilog.warn(0x3200, TAG, 'the clipboard: %{public}s', JSON.stringify(error));
    });
  }

  // The asks the app made, taken each frame and answered.
  private asks: Array<Ask> = [];
  // The ask a picker or the clipboard is answering (its id), if any.
  private pendingPick: number = -1;
  private pendingClipboard: number = -1;

  private showKeyboard(): void {
    // the app's own text field has the focus: the system's keyboard, over it
    focusControl.requestFocus('procraft-typing');
  }

  private hideKeyboard(): void {
    focusControl.requestFocus('procraft-canvas');
  }

  build() {
    Stack() {
      XComponent({
        id: 'procraft-canvas',
        type: XComponentType.SURFACE,
        libraryname: 'procraft',
        controller: this.surface,
      })
        .width('100%')
        .height('100%')
        .onLoad(() => {
          hilog.info(0x3200, TAG, 'the surface is up');
          this.ready = true;
        })
        .onAttach(() => {
          // (said again here: the module is certainly up by now)
          this.tellOwnFolder();
        })
        .onDestroy(() => {
          hilog.info(0x3200, TAG, 'the surface is gone');
          this.ready = false;
        })

      // The keyboard's own field: shown while one of the app's text fields has the focus, so the
      // system's keyboard comes up over the canvas.
      TextInput({ text: this.typed })
        .id('procraft-typing')
        .width(1)
        .height(1)
        .opacity(this.typing ? 1 : 0)
        .focusable(this.typing)
        .onChange((text: string) => {
          if (text.length > 0) {
            procraft.text(text);
            this.typed = '';
          }
        })
        .onSubmit(() => {
          procraft.key(13, 0, true, false);
          procraft.key(13, 0, false, false);
        })
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#121214')
    .onAreaChange((_old: Area, now: Area) => {
      // the page's size in pixels, which the surface is: the app draws at it
      const scale = this.getUIContext().vp2px(1);
      hilog.info(0x3200, TAG, 'the page is %{public}dx%{public}d vp at %{public}f', Number(now.width), Number(now.height), scale);
      const width = Math.round(Number(now.width) * scale);
      const height = Math.round(Number(now.height) * scale);
      hilog.info(0x3200, TAG, 'the page is %{public}dx%{public}d px', width, height);
      procraft.setScale(scale);
      procraft.resized(width, height);
    })
  }

  @State private typed: string = '';
  // The timer that takes the app's asks.
  private pumpTimer: number = -1;

  // The asks the app made, taken and answered each frame.
  private pump(): void {
    const text = procraft.asks();
    if (text === undefined || text.length === 0 || text === '[]') {
      return;
    }
    try {
      this.asks = JSON.parse(text) as Array<Ask>;
    } catch (error) {
      return;
    }
    for (const ask of this.asks) {
      this.answer(ask);
    }
  }
}
