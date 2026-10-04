<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { startEventBridge } from '$lib/ipc/subscribe';
  import { reloadHosts, refreshMetrics, setTrayBehavior } from '$lib/ipc/commands';
  import { theme } from '$lib/stores/theme';
  import { sidebarCollapsed } from '$lib/stores/ui';
  import { streamerMode } from '$lib/stores/streamer';
  import {
    copyOnSelect,
    rightClickPaste,
    serialTimestamps,
    terminalFontSize
  } from '$lib/stores/terminalPrefs';
  import { terminalScheme } from '$lib/stores/terminalScheme';
  import { terminalShortcuts } from '$lib/stores/terminalShortcuts';
  import { loadQuickCommands, quickBarCollapsed, selectedGroup } from '$lib/stores/quickCommands';
  import { refreshInterval, driveMetricsRefresh } from '$lib/stores/settings';
  import { trayBehavior, driveTray } from '$lib/stores/tray';
  import { globalHotkey, applyGlobalHotkey } from '$lib/stores/globalHotkey';
  import { lastError } from '$lib/stores/notifications';

  let { children } = $props();

  onMount(() => {
    let stop: (() => void) | undefined;
    let disposed = false;
    // Reconcile the persisted prefs with their canonical tauri-plugin-store values;
    // the synchronous localStorage mirrors already seeded the first paint (§5.1, §2).
    void theme.hydrate();
    void sidebarCollapsed.hydrate();
    void streamerMode.hydrate();
    void refreshInterval.hydrate();
    void trayBehavior.hydrate();
    void copyOnSelect.hydrate();
    void rightClickPaste.hydrate();
    void serialTimestamps.hydrate();
    void terminalFontSize.hydrate();
    void terminalScheme.hydrate();
    void terminalShortcuts.hydrate();
    void selectedGroup.hydrate();
    void quickBarCollapsed.hydrate();
    void loadQuickCommands();
    // Force a metric refresh on the user's interval; re-arms when the interval changes.
    const stopRefresh = driveMetricsRefresh(() => {
      void refreshMetrics().catch(() => {});
    });
    // The backend knows nothing of the tray until told, so a close before this
    // lands still quits — never a hidden window with no icon to bring it back.
    const stopTray = driveTray(
      (b) => setTrayBehavior(b.minimizeToTray, b.closeToTray),
      (message) => lastError.set(message)
    );
    // Applied once hydration settles on a final value, then again on every later
    // change; the flag keeps the subscription's own immediate fire (with the
    // pre-hydrate mirrored value) from applying twice alongside it.
    let hotkeyHydrated = false;
    const stopHotkey = globalHotkey.subscribe(() => {
      if (hotkeyHydrated) void applyGlobalHotkey();
    });
    void globalHotkey.hydrate().then(() => {
      hotkeyHydrated = true;
      void applyGlobalHotkey();
    });
    // No-op outside Tauri (e.g. a plain `vite preview`); the shell still mounts.
    // Dispose even if the layout unmounts before the subscription resolves. Start
    // the pollers only once listeners are attached, so no status event is missed.
    startEventBridge()
      .then((off) => {
        if (disposed) return off();
        stop = off;
        reloadHosts().catch((err) => lastError.set(err instanceof Error ? err.message : String(err)));
      })
      .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
      stopRefresh();
      stopTray();
      stopHotkey();
    };
  });
</script>

{@render children()}
