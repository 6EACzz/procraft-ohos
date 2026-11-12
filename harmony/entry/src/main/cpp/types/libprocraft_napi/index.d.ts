// ProCraft's own half: the surface the app draws in, and what the system gives it (see
// `cpp/napi_init.cpp`). The interface itself is drawn by the app's Rust code.

export const setOwnFolder: (dir: string) => void;
export const setScale: (scale: number) => void;
export const resized: (width: number, height: number) => void;
export const focused: (focused: boolean) => void;
export const key: (code: number, text: number, pressed: boolean, repeat: boolean) => void;
export const text: (text: string) => void;
export const droppedFile: (path: string) => void;
export const wake: () => void;
export const asks: () => string;
export const answer: (id: number, paths: string, clipboard: string, done: boolean) => void;
// Whether the app has something of its own to close first: BACK closes that instead of the app.
export const canGoBack: () => boolean;
