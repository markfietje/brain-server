export type Platform = 'macos' | 'windows' | 'linux' | 'other';

/**
 * The ONE platform hint the stylesheet keys on (`<html data-platform>`).
 * Native feel policy (D-UX): the webview is ALREADY the OS's (WKWebView /
 * WebView2 / WebKitGTK), so form controls are left NATIVE — the platform
 * renders its own popups, combos, checkboxes and focus halos. Only tokens
 * (type family, radii, accents) adapt per platform; we never re-chrome a
 * control the OS already draws.
 */
export function detectPlatform(): Platform {
	if (typeof navigator === 'undefined') return 'other';
	const ua = navigator.userAgent;
	if (/Mac|iPhone|iPad|iPod/.test(ua)) return 'macos';
	if (/Windows|Win32|WOW64/.test(ua)) return 'windows';
	if (/Linux|X11/.test(ua)) return 'linux';
	return 'other';
}

export function applyPlatformAttr(): void {
	if (typeof document === 'undefined') return;
	document.documentElement.dataset.platform = detectPlatform();
}
