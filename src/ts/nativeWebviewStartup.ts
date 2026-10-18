import { isTauri } from "./platform";
import { fetch as nativeFetch } from "@tauri-apps/plugin-http";
import { downloadFile, openURL } from "./globalApi.svelte";
import { alertError } from "./alert";
import { createWebviewDownloadHandler, installWebviewBridge } from "./webviewDownload";

export function installNativeWebviewHandling() {
  if (!isTauri) return;
  const save = createWebviewDownloadHandler({ fetch: nativeFetch, save: downloadFile, reportError: error => alertError(String(error)) });
  const lifetime = new AbortController();
  window.addEventListener('pagehide', () => lifetime.abort(), { once: true });
  installWebviewBridge((name, source) => save(name, source, lifetime.signal), console.error, openURL,
    // The development-only Sionyw login receives its result through the popup's postMessage.
    url => import.meta.env.DEV && url.origin === 'https://account.sionyw.com'
      && url.searchParams.get('response_mode') === 'web_message');
}
