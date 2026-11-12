// The Tauri bridge: commands the Rust side answers and events it sends.
const tauri = window.__TAURI__;

export const invoke = (cmd, args) => tauri.core.invoke(cmd, args);
export const listen = (event, fn) => tauri.event.listen(event, (e) => fn(e.payload));
