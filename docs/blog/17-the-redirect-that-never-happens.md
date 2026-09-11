# The redirect that never happens

*2026-09-11. The plugin transport now refuses to follow redirects at all.
This post explains the credential class behind that decision.*

When an HTTP client follows a redirect, it re-sends the request to a new
URL. The question is which headers travel along. Browsers strip
`Authorization` across origins. That sounds sufficient until you list what
real SDKs authenticate with: `X-API-Key`, `Private-Token`, `X-Auth-Token`,
custom bearer schemes. Standard denylists do not cover them, and 2026
produced the CVEs to prove it: custom auth headers forwarded across
origins in widely used clients, fixed only by moving to allowlists.

The safe posture is to never find out what your client forwards. The
plugin transport sends `redirect: "manual"` and treats any 3xx as a
refusal. A redirect from your own server is not followed either, because
a compromised or DNS-rebound server turning a 200 into a 302 toward an
attacker host is exactly the shape that harvests bearers. The failure
mode is a loud network error, and the operator investigates a redirect
that should not exist instead of rotating a token that already leaked.

Fail closed on the transport, argue about convenience later. Credentials
are easier to keep than to revoke.
