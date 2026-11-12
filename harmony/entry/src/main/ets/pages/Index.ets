// ProCraft on the tablet: the page the app shows. The interface itself is the desktop's own,
// drawn by the app's Rust code into the XComponent's surface (see `procraft/desktop/src/
// ohos_main.rs`); this page is what the system talks to — the surface, the file picker, the
// clipboard, the soft keyboard — and passes it on.

import { fileIo as fs } from '@kit.CoreFileKit';
import { picker } from '@kit.CoreFileKit';
import { pasteboard } from '@kit.BasicServicesKit';
import { image } from '@kit.ImageKit';
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

// What a clipboard write of a picture says: the picture is the app's own, written as a PNG in the
// app's folder, and only its path crosses (its pixels as text would be tens of megabytes).
interface PictureDetails {
  width: number;
  height: number;
  file: string;
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
    // nothing is left unanswered: the app waits on every ask it made, and this page is the only
    // one that could answer them (an unanswered ask holds the app up until its own timeout)
    this.answerEverythingPending();
    procraft.focused(false);
  }

  // Answers every ask still waiting, so the app is never left waiting on this page: the ones a
  // picker or the clipboard was about to answer, and anything still queued.
  private answerEverythingPending(): void {
    const pending: Array<number> = [];
    // every ask a picker or the clipboard is still working on, not just the last one: two picks
    // in a row used to leave the first one waiting on an answer that was never sent
    this.pendingPick.forEach((_: number, id: number) => {
      pending.push(id);
    });
    this.pendingPick.clear();
    this.pendingClipboard.forEach((_: number, id: number) => {
      pending.push(id);
    });
    this.pendingClipboard.clear();
    try {
      const text = procraft.asks();
      if (text !== undefined && text !== null && text.length > 0 && text !== '[]') {
        const asked = JSON.parse(text) as Array<Ask>;
        for (const ask of asked) {
          pending.push(ask.id);
        }
      }
    } catch (error) {
      // (the page is going: nothing more to do about a queue that can't be read)
    }
    for (const id of pending) {
      try {
        procraft.answer(id, '', '', false);
      } catch (error) {
        // (the app is going too)
      }
    }
  }

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
        this.pendingPick.set(ask.id, ask.id);
        this.pick(ask.id, details as PickDetails);
        break;
      case 'say':
        this.say(details as SayDetails);
        procraft.answer(ask.id, '', '', true);
        break;
      case 'clipboard-read':
        this.pendingClipboard.set(ask.id, ask.id);
        this.readClipboard(ask.id);
        break;
      case 'clipboard-write-text':
        this.writeClipboardText((details as WriteTextDetails).text);
        procraft.answer(ask.id, '', '', true);
        break;
      case 'clipboard-write-picture':
        // the app's own PNG, put on the clipboard: it answers once it is really there, so a
        // paste of something that never made it is told so rather than pasted empty
        this.writeClipboardPicture(ask.id, details as PictureDetails);
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
  // The app only ever reads and writes plain paths, and what the picker hands back is a URI that
  // belongs to the system's own folders: so nothing is answered with a URI. What is picked is
  // copied into this app's sandbox and the copy's path is answered; what the app writes for a
  // pick that saves is copied out to the URI afterwards (see `drainOutbox`).
  private async pick(id: number, ask: PickDetails): Promise<void> {
    try {
      if (ask.folder) {
        const options = new picker.DocumentSelectOptions();
        // it asked for a folder, so the picker must offer folders: without this it opens on files
        // and can only hand back a file where a folder was wanted (layers exported as PNGs go to
        // a folder of their own, so this is the difference between export working and not)
        options.selectMode = picker.DocumentSelectMode.FOLDER;
        options.maxSelectNumber = 1;
        const chosen = new picker.DocumentViewPicker(this.context());
        const picked = await chosen.select(options);
        if (picked.length === 0) {
          // (cancelled: answered as none, so the app can say so and stop waiting)
          this.answerPaths(id, []);
          return;
        }
        // a folder of this app's own stands in for the one the user chose: what the app writes
        // there is copied out to the real one as it appears
        const target = this.ownFolder(`outbox/${Date.now()}-${this.stepped()}`);
        this.outboxDirs.set(target, picked[0]);
        this.answerPaths(id, [target]);
        return;
      }
      if (ask.save) {
        const name = ask.name ?? 'untitled';
        const options = new picker.DocumentSaveOptions();
        options.newFileNames = [name];
        const chosen = new picker.DocumentViewPicker(this.context());
        const saved = await chosen.save(options);
        if (saved.length === 0) {
          this.answerPaths(id, []);
          return;
        }
        // the app writes a name of its own and that write is copied out to the name the user
        // chose: the app may save to the same path again later without asking again, so the
        // copy-out stays registered for as long as the page lives
        const target = `${this.ownFolder('outbox')}/${Date.now()}-${this.stepped()}-${this.nameOf(name)}`;
        this.outboxFiles.set(target, saved[0]);
        this.answerPaths(id, [target]);
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
      // each is copied in on its own: one that can't be read (a folder, a file already gone) must
      // not lose the others, and an empty answer already means "cancelled" to the app
      const paths: Array<string> = [];
      for (let at = 0; at < picked.length; at++) {
        try {
          paths.push(this.copyIn(picked[at]));
        } catch (error) {
          hilog.warn(0x3200, TAG, 'the pick %{public}s couldn\'t be read: %{public}s',
            picked[at], this.why(error));
        }
      }
      if (picked.length > 0 && paths.length === 0) {
        // everything it picked failed to copy: telling the app "cancelled" would be a lie that
        // closes its dialog as if the user had chosen nothing, which is what a failure is not
        this.answerPaths(id, []);
        return;
      }
      this.answerPaths(id, paths);
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the picker: %{public}s', this.why(error));
      this.answerPaths(id, []);
    }
  }

  // A counter that keeps two names made in the same millisecond apart.
  private stepped(): number {
    this.steps = this.steps + 1;
    return this.steps;
  }

  // The last part of a URI or a path: what a file is called.
  private nameOf(uriOrPath: string): string {
    const at = uriOrPath.lastIndexOf('/');
    const name = at >= 0 ? uriOrPath.substring(at + 1) : uriOrPath;
    return name.length > 0 ? name : 'file';
  }

  // A folder of this app's own, made if it is not there yet.
  private ownFolder(name: string): string {
    const dir = `${this.context().filesDir}/${name}`;
    try {
      fs.mkdirSync(dir, true);
    } catch (error) {
      // (it is there: the app is given it either way)
    }
    return dir;
  }

  // One file picked, copied into this app's sandbox, and the copy's path.
  private copyIn(uri: string): string {
    const dest = `${this.ownFolder('picked')}/${Date.now()}-${this.stepped()}-${this.nameOf(uri)}`;
    // the URI can be opened and what is open can be copied; the app is handed the copy, which is
    // a plain path in its own sandbox
    const file = fs.openSync(uri, fs.OpenMode.READ_ONLY);
    try {
      fs.copyFileSync(file.fd, dest);
    } finally {
      fs.closeSync(file);
    }
    hilog.info(0x3200, TAG, 'took %{public}s in as %{public}s', uri, dest);
    return dest;
  }

  // The paths picked, as the app reads them (each on its own line); none, cancelled.
  private answerPaths(id: number, paths: Array<string>): void {
    const text = paths.join('\n');
    this.pendingPick.delete(id);
    procraft.answer(id, text, '', true);
  }

  private say(details: SayDetails): void {
    // the page's own prompt action, not the global one: the global one is not bound to this
    // window and is on its way out of the SDK
    this.getUIContext().getPromptAction().showDialog({
      title: details.title,
      message: details.text,
      buttons: [{ text: 'OK', color: '#4F46E5' }],
    }).catch((error: BusinessError) => {
      hilog.warn(0x3200, TAG, 'the message: %{public}s', JSON.stringify(error));
    });
  }

  // The app's own picture, put on the system's clipboard. It is a PNG in the app's folder: the
  // pixels are read back and given to the clipboard as a picture, and the answer says whether
  // they really made it there.
  private async writeClipboardPicture(id: number, details: PictureDetails): Promise<void> {
    let answered = false;
    try {
      const file = fs.openSync(details.file, fs.OpenMode.READ_ONLY);
      try {
        const source = image.createImageSource(file.fd);
        try {
          const pixels = await source.createPixelMap();
          try {
            const data = pasteboard.createData(pasteboard.MIMETYPE_PIXELMAP, pixels);
            await pasteboard.getSystemPasteboard().setData(data);
            answered = true;
            procraft.answer(id, '', '', true);
            hilog.info(0x3200, TAG, 'the picture is on the clipboard, %{public}s',
              `${details.width}x${details.height}`);
          } finally {
            // (the pixels are let go of whether or not they made it onto the clipboard)
            try {
              await pixels.release();
            } catch (error) {
            }
          }
        } finally {
          try {
            await source.release();
          } catch (error) {
          }
        }
      } finally {
        try {
          fs.closeSync(file);
        } catch (error) {
        }
      }
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the picture on the clipboard: %{public}s', this.why(error));
    }
    if (!answered) {
      // the app is told the picture did not make it, so it can say so instead of the paste
      // coming back empty for no reason it can see
      procraft.answer(id, '', '', false);
    }
  }

  // What the system's clipboard holds, as the app reads it.
  private async readClipboard(id: number): Promise<void> {
    try {
      const board = pasteboard.getSystemPasteboard();
      const data = await board.getData();
      // a picture first, then files, then words
      if (data.hasType(pasteboard.MIMETYPE_PIXELMAP)) {
        const pixels = data.getPrimaryPixelMap();
        if (pixels !== undefined) {
          // the picture goes into the app's own folder as PNG, and the app is told its path: a
          // picture's pixels as text would be tens of megabytes of hex, which the app would spend
          // seconds parsing on its one thread (and a photograph would not fit at all)
          const path = `${this.context().filesDir}/clipboard-${Date.now()}.png`;
          // (the packer takes an open file's descriptor, not a path: it writes through it)
          const file = fs.openSync(path, fs.OpenMode.READ_WRITE | fs.OpenMode.CREATE | fs.OpenMode.TRUNC);
          const packer = image.createImagePacker();
          try {
            const options: image.PackingOption = { format: 'image/png', quality: 100 };
            await packer.packToFile(pixels, file.fd, options);
            this.answerClipboard(id, `picture-file:${path}`);
          } finally {
            // (each let go of on its own: one failing to be released must not keep the file open,
            // and an uncaught promise here would stop the process from inside the pump's timer)
            try {
              await packer.release();
            } catch (error) {
            }
            try {
              // a full-screen picture is twenty megabytes of the clipboard's own: letting go of it
              // is what keeps a paste from one gesture to the next from holding all of them
              await pixels.release();
            } catch (error) {
            }
            try {
              fs.closeSync(file);
            } catch (error) {
            }
          }
          return;
        }
      }
      if (data.hasType(pasteboard.MIMETYPE_TEXT_URI)) {
        const uri = data.getPrimaryText();
        if (uri !== undefined && uri.length > 0) {
          // the app reads plain paths, so what it is given is the file copied into its own
          // sandbox, not the URI the clipboard holds
          const paths: Array<string> = [];
          for (const one of uri.split('\n')) {
            if (one.length > 0) {
              try {
                paths.push(this.copyIn(one));
              } catch (error) {
                hilog.warn(0x3200, TAG, 'the pasted %{public}s couldn\'t be read: %{public}s',
                  one, this.why(error));
              }
            }
          }
          this.answerClipboard(id, paths.length > 0 ? `files:${paths.join('\n')}` : 'nothing');
          return;
        }
      }
      const text = data.getPrimaryText();
      this.answerClipboard(id, text !== undefined && text.length > 0 ? `text:${text}` : 'nothing');
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the clipboard: %{public}s', this.why(error));
      this.answerClipboard(id, 'nothing');
    }
  }

  private answerClipboard(id: number, clipboard: string): void {
    this.pendingClipboard.delete(id);
    procraft.answer(id, '', clipboard, true);
  }

  private writeClipboardText(text: string): void {
    const data = pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, text);
    pasteboard.getSystemPasteboard().setData(data).catch((error: BusinessError) => {
      hilog.warn(0x3200, TAG, 'the clipboard: %{public}s', JSON.stringify(error));
    });
  }

  // The asks the app made, taken each frame and answered.
  private asks: Array<Ask> = [];
  // The asks a picker or the clipboard is answering, by id: a second pick used to overwrite the
  // first, whose ask then waited on an answer that never came (the app hangs until its timeout).
  private pendingPick: Map<number, number> = new Map<number, number>();
  private pendingClipboard: Map<number, number> = new Map<number, number>();
  // The names picked for what the app saves, and for folders it writes a set of files into: the
  // app writes a path of this app's own making, and what appears there is copied out to the
  // folder the user actually chose. Keyed by this app's own path, valued by the URI.
  private outboxFiles: Map<string, string> = new Map<string, string>();
  private outboxDirs: Map<string, string> = new Map<string, string>();
  // What each of those was last copied at, so a file written again is copied again and one that
  // has not changed is not (a save that writes the same picture over and over would otherwise
  // copy every 16 ms).
  private outboxSizes: Map<string, number> = new Map<string, number>();
  // Two names made in the same millisecond need to differ.
  private steps: number = 0;

  private showKeyboard(): void {
    // the app's own text field has the focus: the system's keyboard, over it. The field is always
    // focusable and always in the tree (it is one pixel and invisible until now), so the focus can
    // be asked for here rather than after a redraw — asking before the field could be focused threw
    // 150001 and, uncaught inside a timer, stopped the whole process as a Js Error
    this.typing = true;
    try {
      this.getUIContext().getFocusController().requestFocus('procraft-typing');
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the keyboard: %{public}s', this.why(error));
    }
  }

  private hideKeyboard(): void {
    // the canvas takes the focus back, which takes the keyboard away
    try {
      this.getUIContext().getFocusController().requestFocus('procraft-canvas');
    } catch (error) {
      hilog.warn(0x3200, TAG, 'the keyboard away: %{public}s', this.why(error));
    }
  }

  // What the app wrote where it was told to write is copied out to where the user asked for it:
  // it saves a path of this app's own making, and each save is a new file to copy out.
  private drainOutbox(): void {
    this.outboxFiles.forEach((uri: string, sandbox: string) => {
      try {
        if (!fs.accessSync(sandbox)) {
          return;
        }
        // `.part` is the app's own half-written name (it writes that and renames onto the real
        // name): copying it would put a half-written picture where the user's file is, so it is
        // left alone and the real name is copied when the rename makes it appear
        if (sandbox.endsWith('.part')) {
          return;
        }
        const at = fs.statSync(sandbox);
        const size = at.size;
        if (this.outboxSizes.get(sandbox) === size) {
          return;
        }
        this.copyOut(sandbox, uri);
        this.outboxSizes.set(sandbox, size);
      } catch (error) {
        hilog.warn(0x3200, TAG, 'out %{public}s: %{public}s', sandbox, this.why(error));
      }
    });
    this.outboxDirs.forEach((uri: string, sandbox: string) => {
      try {
        if (!fs.accessSync(sandbox)) {
          return;
        }
        for (const name of fs.listFileSync(sandbox)) {
          if (name.endsWith('.part')) {
            continue;
          }
          const from = `${sandbox}/${name}`;
          const at = fs.statSync(from);
          if (this.outboxSizes.get(from) === at.size) {
            continue;
          }
          this.copyOut(from, `${uri}/${name}`);
          this.outboxSizes.set(from, at.size);
        }
      } catch (error) {
        hilog.warn(0x3200, TAG, 'out of %{public}s: %{public}s', sandbox, this.why(error));
      }
    });
  }

  // One file of this app's, written where the user asked for it: the URI is opened and what it
  // is open to is overwritten with the file's own bytes.
  private copyOut(sandbox: string, uri: string): void {
    const file = fs.openSync(uri, fs.OpenMode.WRITE_ONLY | fs.OpenMode.CREATE | fs.OpenMode.TRUNC);
    try {
      fs.copyFileSync(sandbox, file.fd);
    } finally {
      fs.closeSync(file);
    }
    hilog.info(0x3200, TAG, 'put %{public}s out as %{public}s', sandbox, uri);
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
        // the app's own keys are the canvas's: a key pressed with a keyboard (or the system's
        // BACK, which arrives as Escape) is the app's to act on first
        .focusable(true)
        .defaultFocus(true)
        // The keys themselves are not sent from here, and neither is what an ordinary key types:
        // the surface's own callback already hands the app every key and whether it went down or
        // up (see `OnKey` in napi_init.cpp), and the app works the character of a letter, digit
        // or punctuation mark out of the key's code (see `key_of` in ohos_main.rs). Sending them
        // from here as well would type everything twice.
        // What that path cannot carry is a character from outside ASCII — a composed CJK glyph,
        // an accent — which is what a keyboard of the system's own gives and no key code spells.
        .onKeyEvent((event: KeyEvent) => {
          if (event.type === KeyType.Down && event.keyText.length === 1
            && event.keyText.charCodeAt(0) > 127) {
            procraft.text(event.keyText);
          }
        })
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
        // always focusable, though: whether it can be focused must not depend on a redraw having
        // happened by the time the focus is asked for, and a one-pixel field that is invisible
        // costs nothing while no keyboard is up
        .focusable(true)
        // what the keyboard actually enters, rather than the field's whole text: the field keeps
        // nothing (every insert clears it), so its text saying nothing is not a keyboard saying
        // nothing — and a backspace takes text away instead of adding it, which a comparison of
        // the field's text before and after cannot see at all
        .onDidInsert((value: InsertValue) => {
          if (value.insertValue.length > 0) {
            procraft.text(value.insertValue);
          }
        })
        .onDidDelete((_value: DeleteValue) => {
          // the app is told a backspace in the one way it understands (U+0008 is what it reads
          // as Backspace); a forward delete is told as the key it is
          procraft.text('\u0008');
        })
        .onSubmit(() => {
          // 2054 is the system's KEYCODE_ENTER, which is what the app knows as Enter: a 13 here
          // would be a code it has never heard of and would drop
          procraft.key(2054, 0, true, false);
          procraft.key(2054, 0, false, false);
        })
        .onChange((text: string) => {
          // (the field is emptied as it is used: it is only a way to reach the system's keyboard)
          if (text.length > 0) {
            this.typed = '';
          }
        })
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#121214')
    .onAreaChange((_old: Area, now: Area) => {
      // the page's size in pixels, which the surface is: the app draws at it
      const scale = this.getUIContext().vp2px(1);
      hilog.info(0x3200, TAG, 'the page is %{public}sx%{public}s at %{public}s',
        String(Number(now.width)), String(Number(now.height)), scale.toFixed(2));
      const width = Math.round(Number(now.width) * scale);
      const height = Math.round(Number(now.height) * scale);
      hilog.info(0x3200, TAG, 'the page is %{public}dx%{public}d px', width, height);
      procraft.setScale(scale);
      procraft.resized(width, height);
    })
  }

  // BACK is the app's whenever it has something of its own to close: it is given as the Escape
  // the app knows, and the system is told not to take the page away. With nothing open, BACK is
  // the system's own again, and leaves the app.
  onBackPress(): boolean {
    try {
      if (procraft.canGoBack()) {
        procraft.key(2, 0, true, false);
        procraft.key(2, 0, false, false);
        return true;
      }
    } catch (error) {
      hilog.warn(0x3200, TAG, 'back: %{public}s', this.why(error));
    }
    return false;
  }

  @State private typed: string = '';
  // The timer that takes the app's asks.
  private pumpTimer: number = -1;

  // The asks the app made, taken and answered each frame.
  private pump(): void {
    // what the app wrote where it was told to is copied out to where the user asked: it saves a
    // path of this page's own making and never touches the user's folder itself
    try {
      this.drainOutbox();
    } catch (error) {
      hilog.warn(0x3200, TAG, 'out: %{public}s', this.why(error));
    }
    const text = procraft.asks();
    if (text === undefined || text === null || text.length === 0 || text === '[]') {
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
