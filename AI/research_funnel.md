# Research: Tailscale Funnel in front of soos-remote

Status: research note for GitHub #339 (branch `feat/remote-funnel-passkey`), 2026-10-06.
Scope: the facts the Funnel + passkey design (owner decisions D-A to D-I) depends on. Nothing on
the host was changed; only read-only commands were run (`tailscale version`,
`tailscale serve status`, `tailscale funnel status`, `tailscale status --self --json`).

## 0. Version and sources

- Installed client: `tailscale version` → `1.102.4`, commit `3caf7d9e7dca…` (`-dirty`, Arch
  package build), Go `go1.27.0`. `3caf7d9e` is the annotated tag object of `v1.102.4`
  (`git ls-remote` shows `refs/tags/v1.102.4 → 3caf7d9e…`, `^{}` → commit
  `bbcd7d1fc2054b9189ebc1531acf74bd880ca0c8`), so the source read below is exactly the installed
  release (Arch may carry packaging patches; none touch the files below as far as the package
  is a plain build of the tag).
- Source permalinks use `https://github.com/tailscale/tailscale/blob/bbcd7d1fc2054b9189ebc1531acf74bd880ca0c8/<path>#L<n>`
  (abbreviated below as `ts@bbcd7d1:<path>:<line>`).
- Go standard library: `net/http/httputil/reverseproxy.go` of the local toolchain (Go 1.27.1,
  same behaviour as the 1.27.0 used to build tailscaled for the lines quoted).
- Docs: https://tailscale.com/kb/1223/funnel , https://tailscale.com/kb/1312/serve ,
  https://tailscale.com/kb/1247/funnel-serve-use-cases , Public Suffix List
  (https://github.com/publicsuffix/list, `public_suffix_list.dat`).

Current host state (read-only): `tailscale serve status` and `tailscale funnel status` both show
`https://<node>.<tailnet>.ts.net (tailnet only)` with `/ proxy unix:/run/user/1000/soos-remote/remote.sock`.
`tailscale status --self --json` shows the node already holds the `https` and `funnel`
capabilities and `https://tailscale.com/cap/funnel-ports?ports=443,8443,10000`; MagicDNS is on
and one cert domain is provisioned. So the prerequisites of section 3 are already met on this
tailnet.

## 1. Funnel to a Unix socket, ports, coexistence with serve

### 1.1 `tailscale funnel --bg unix:<path>` is supported

- `tailscale serve` and `tailscale funnel` share one implementation, `runServeCombined`
  (`ts@bbcd7d1:cmd/tailscale/cli/serve_v2.go:372`); the only difference is the `funnel` bool,
  which (a) runs `verifyFunnelEnabled` (`:408-412`) and (b) calls `applyFunnel` →
  `ServeConfig.SetFunnel(dnsName, srvPort, true)` (`:1021-1025`, `:1350-1362`).
- The shared help text, printed for both subcommands (`serveHelpCommon`, `:137-157`), says:
  "On Unix-like systems, you can also specify a Unix domain socket (e.g.,
  unix:/tmp/myservice.sock)" with the example `tailscale %[1]s unix:/var/run/myservice.sock`
  (`%[1]s` is `serve` or `funnel`).
- In tailscaled the web handler is the same for both: `proxyHandlerForBackend` handles the
  `unix:` scheme (`ts@bbcd7d1:ipn/ipnlocal/serve.go:869-886`), refuses the tailscaled socket
  itself (`ErrProxyToTailscaledSocket`), and dials `net.Dialer.DialContext(ctx, "unix", socketPath)`
  (`:1001-1008`). There is no Funnel-specific branch in the proxy.
- Consequence for soos-remote: the service unit can keep `RestrictAddressFamilies=AF_UNIX`;
  Funnel needs no network socket in the service (D-G holds).
- Upstream request as seen by the backend over the Unix socket: HTTP/1.1 (plain `http://` to a
  Unix socket; h2c is used only for gRPC content types, `:1049-1055`), `Host: localhost`
  (`:974-978`: "For Unix sockets, use the URL's host (localhost) instead of the incoming host"),
  the public host only in `X-Forwarded-Host` (section 2).

### 1.2 Ports

- KB 1223: "Funnel can only listen on ports `443`, `8443`, and `10000`".
- Code: the allowed set is not hard-coded in the client; it comes from the control-plane node
  capability `https://tailscale.com/cap/funnel-ports?ports=...` (`ts@bbcd7d1:tailcfg/tailcfg.go:2563-2567`),
  checked by `ipn.CheckFunnelPort` (`ts@bbcd7d1:ipn/serve.go:664-745`). On this node the cap is
  `ports=443,8443,10000`.
- CLI quirk: `runServeCombined` always checks port 443 (`verifyFunnelEnabled(ctx, 443)`,
  `serve_v2.go:410`) regardless of `--https=<port>`; irrelevant here because 443 is allowed.
- Funnel requires HTTPS/TLS (KB 1223: "Funnel only works over TLS-encrypted connections").

### 1.3 Serve and Funnel on the same node/port

- Funnel is a per-`host:port` flag (`ServeConfig.AllowFunnel map[HostPort]bool`,
  `ipn/serve.go:514-531`) on top of the ordinary serve handler for that port. There is ONE
  handler table per port (`ServeConfigView.FindTCP(port)`, `ipn/serve.go:905-911`), used both by
  direct tailnet connections and by Funnel ingress connections
  (`tcpHandlerForServe`, `ipn/ipnlocal/serve.go:626-643`). So a port is either tailnet-only or
  tailnet + public; it cannot be split by audience with different backends.
- KB 1312: "The same port number cannot be used for Serve (available only within the tailnet)
  and Funnel (available within the tailnet and to the public) at the same time." Whichever
  command configured the port last decides.
- Therefore, two valid deployments:
  - (A) `funnel` on 443 to the soos-remote socket: the same URL serves both audiences.
  - (B) keep `serve` 443 tailnet-only and add `funnel --https=8443` (or 10000) to the same
    socket: the public URL is `https://<node>.<tailnet>.ts.net:8443`.
  In both cases the backend distinguishes the audience per request from the headers (section 2),
  not from the port; the design must not rely on the port.

### 1.4 Tailnet clients keep identity headers when Funnel is on

Yes. A tailnet client that resolves the name through MagicDNS connects directly to the node's
Tailscale IP; that connection is built with `funnelFlow == nil` (`tcpHandlerForServe(ap.Port(), srcAddr, nil)`, `ipn/ipnlocal/serve.go:138`,
from the local listener path), so `addTailscaleIdentityHeaders` takes the `WhoIs` branch and sets
`Tailscale-User-Login` (section 2). Only connections that arrive through the Funnel ingress
PeerAPI endpoint (`/v0/ingress`, `handleServeIngress` → `HandleIngressTCPConn`,
`ipn/ipnlocal/serve.go:444-500`, `:1480-1543`) carry a non-nil `funnelFlow`.

Practical note for the iPhone: with the Tailscale app connected (and "Use Tailscale DNS" on) the
name resolves to the 100.x address and the request is a tailnet request with identity; with the
VPN off, public DNS resolves the name to Tailscale's Funnel relays and the request is a Funnel
request without identity. The same page must therefore handle both paths.

## 2. Headers set and stripped by tailscaled

### 2.1 The code (installed version)

`ipn/ipnlocal/serve.go:959-986` (`reverseProxy.ServeHTTP`) uses `httputil.ReverseProxy` with a
`Rewrite` hook that calls, in order, `addProxyForwardedHeaders(r)`,
`rp.lb.addTailscaleIdentityHeaders(r)`, `rp.lb.addAppCapabilitiesHeader(r)`.

Go `ReverseProxy` with `Rewrite` first strips client forwarding headers
(`net/http/httputil/reverseproxy.go:487-494`):

```go
if p.Rewrite != nil {
	// Strip client-provided forwarding headers.
	outreq.Header.Del("Forwarded")
	outreq.Header.Del("X-Forwarded-For")
	outreq.Header.Del("X-Forwarded-Host")
	outreq.Header.Del("X-Forwarded-Proto")
```

Then tailscaled sets them (`ts@bbcd7d1:ipn/ipnlocal/serve.go:1068-1076`):

```go
func addProxyForwardedHeaders(r *httputil.ProxyRequest) {
	r.Out.Header.Set("X-Forwarded-Host", r.In.Host)
	if r.In.TLS != nil {
		r.Out.Header.Set("X-Forwarded-Proto", "https")
	}
	if c, ok := serveHTTPContextKey.ValueOk(r.Out.Context()); ok {
		r.Out.Header.Set("X-Forwarded-For", c.SrcAddr.Addr().String())
	}
}
```

And the identity headers (`ts@bbcd7d1:ipn/ipnlocal/serve.go:1078-1107`):

```go
func (b *LocalBackend) addTailscaleIdentityHeaders(r *httputil.ProxyRequest) {
	// Clear any incoming values squatting in the headers.
	r.Out.Header.Del("Tailscale-User-Login")
	r.Out.Header.Del("Tailscale-User-Name")
	r.Out.Header.Del("Tailscale-User-Profile-Pic")
	r.Out.Header.Del("Tailscale-Funnel-Request")
	r.Out.Header.Del("Tailscale-Headers-Info")

	c, ok := serveHTTPContextKey.ValueOk(r.Out.Context())
	if !ok {
		return
	}
	if c.Funnel != nil {
		r.Out.Header.Set("Tailscale-Funnel-Request", "?1")
		return
	}
	node, user, ok := b.WhoIs("tcp", c.SrcAddr)
	if !ok {
		return // traffic from outside of Tailnet (funneled or local machine)
	}
	if node.IsTagged() {
		// 2023-06-14: Not setting identity headers for tagged nodes.
		// Only currently set for nodes with user identities.
		return
	}
	r.Out.Header.Set("Tailscale-User-Login", encTailscaleHeaderValue(user.LoginName))
	r.Out.Header.Set("Tailscale-User-Name", encTailscaleHeaderValue(user.DisplayName))
	r.Out.Header.Set("Tailscale-User-Profile-Pic", user.ProfilePicURL)
	r.Out.Header.Set("Tailscale-Headers-Info", "https://tailscale.com/s/serve-headers")
}
```

`addAppCapabilitiesHeader` (`:1124-1155`) always deletes `Tailscale-App-Capabilities` and never
sets it for Funnel (`if !ok || c.Funnel != nil { return nil }`); soos-remote does not use it.

### 2.2 Resulting header table (what soos-remote receives)

| Header | Tailnet request (user-owned node) | Funnel request (public internet) |
|---|---|---|
| `Tailscale-User-Login` | set by tailscaled (Q-encoded if non-ASCII, empty if invalid UTF-8) | **deleted, never set** |
| `Tailscale-User-Name`, `-Profile-Pic`, `Tailscale-Headers-Info` | set | deleted, never set |
| `Tailscale-Funnel-Request` | deleted (absent) | set to `?1` (structured-field boolean) |
| `Tailscale-App-Capabilities` | only if `--accept-app-caps` configured (not used) | deleted |
| `X-Forwarded-Host` | `r.In.Host` (client-sent `Host`) | `r.In.Host` (client-sent `Host`) |
| `X-Forwarded-Proto` | `https` (TLS listener) | `https` |
| `X-Forwarded-For` | client Tailscale IP | public client IP from `Tailscale-Ingress-Src`, supplied by Tailscale's ingress relay |
| `Forwarded` | deleted | deleted |
| `Host` | `localhost` (Unix-socket backend) | `localhost` |

Tagged tailnet nodes and connections from the local machine get neither identity headers nor
`Tailscale-Funnel-Request` (the `WhoIs`/`IsTagged` early returns): soos-remote must treat
"no identity header" as unauthenticated regardless of the Funnel marker.

KB 1312 confirms: "Funnel traffic, which is publicly available, does not include identity
headers", identity headers "are not populated for traffic originating from tagged devices", and
"If Serve finds the following headers on an incoming request, it will remove them for security
reasons, to avoid header spoofing." `Tailscale-Funnel-Request` is not documented in the KB; it
exists only in code, so it is a defence-in-depth signal, not the primary one.

### 2.3 Spoofing analysis

- A Funnel client cannot inject `Tailscale-User-Login`: the deletion runs unconditionally before
  the Funnel early return, and `Header.Del` uses the canonical key (Go canonicalises every
  incoming header name, so `tailscale-user-login`, `TAILSCALE-USER-LOGIN`, and repeated copies
  are all removed).
- Header names that differ in spelling are not removed (for example `Tailscale_User_Login` with
  underscores is a valid token and is forwarded). soos-remote must match only the exact
  hyphenated name, case-insensitively, and never normalise `_` to `-`.
- Client-supplied `X-Real-IP` and any other non-listed header pass through unmodified; soos-remote
  must not read them.
- `X-Forwarded-Host` is the client-controlled `Host` header, not the TLS SNI. tailscaled routes
  by SNI (`getServeHandler` uses `r.TLS.ServerName` for TLS requests,
  `ipn/ipnlocal/serve.go:807-818`; certificates are chosen by SNI, `:1341-1366`), but forwards
  `Host` unchanged. A non-browser client can therefore send any `X-Forwarded-Host` value through
  Funnel. The existing host check (ADR 2026-10-05, commit e3a897b) is a DNS-rebinding / browser
  guard, not an authentication; for passkeys the security binding is the WebAuthn
  `clientDataJSON.origin` and `rpIdHash` checks, which the browser, not the client network
  request, controls.
- `X-Forwarded-For` on Funnel is the public client address reported by the Tailscale ingress
  relay (a peer with the ingress capability, `canIngress`, `ipn/ipnlocal/peerapi.go:595-597`).
  It is plausibly accurate but attacker-chosen at scale (IPv6, botnets); use it at most as a
  rate-limit bucket hint, never for authorization, and keep global (not per-IP) bounds.

## 3. Funnel prerequisites, limits

From KB 1223 and code:

- Tailscale v1.38.3 or later; MagicDNS enabled; HTTPS certificates enabled for the tailnet.
- Tailnet policy `nodeAttrs` with `"attr": ["funnel"]` targeting the node/user, e.g.
  `{"target": ["autogroup:member"], "attr": ["funnel"]}`. Enforced in code by `NodeCanFunnel`
  (`ipn/serve.go:652-662`): requires both `tailcfg.CapabilityHTTPS` and `NodeAttrFunnel`
  (`"funnel"`, `tailcfg/tailcfg.go:2576-2577`).
- Funnel is refused while shields-up is on (`setServeConfigLocked`,
  `ipn/ipnlocal/serve.go:329-332`).
- Available on all plans, including the free Personal plan (KB 1223: "available for all plans")
  — consistent with D-A (no cost, no domain).
- Ports 443, 8443, 10000 only (section 1.2).
- "Traffic sent over a Funnel is subject to non-configurable bandwidth limits" (amount not
  published). soos-remote traffic (small JSON, one SSE stream) is far below any plausible limit.
- Public DNS for the node name can take up to 10 minutes to appear after enabling Funnel.
- TLS terminates in tailscaled on the PC; relays forward encrypted TCP ("Funnel relay servers do
  not decrypt the traffic"). The certificate is a Let's Encrypt cert for
  `<node>.<tailnet>.ts.net`; frequent re-issuance can hit Let's Encrypt rate limits (KB mentions
  waits of up to 34 hours).
- Funnel config persists in the serve config (background `--bg`) and survives restarts; turning
  it off is `tailscale funnel --https=<port> off` (owner action, D-I).

### 3.1 WebAuthn-relevant DNS facts

- `ts.net` is on the Public Suffix List (section "Tailscale Inc.", entry `ts.net`). The WebAuthn
  RP ID must therefore be the full host `<node>.<tailnet>.ts.net` (an RP ID equal to a public
  suffix is rejected by browsers). `<tailnet>.ts.net` is not itself a public suffix, but using
  the full node host is the narrowest correct choice.
- The RP ID is a host name without port: a passkey registered on
  `https://<node>.<tailnet>.ts.net` (tailnet, port 443) is valid for an assertion on
  `https://<node>.<tailnet>.ts.net:8443` (Funnel on 8443) and vice versa. The origin check in
  the verifier must then accept exactly the configured origin(s) (`https://host` and, for
  deployment (B), `https://host:8443`), nothing else.
- The tailnet path and the Funnel path use the same name and certificate, so a passkey enrolled
  over the tailnet (D-E) is usable over Funnel (D-B/D-C) without re-registration.

## 4. Risk notes

1. Public exposure and scanning. The node name is already public in Certificate Transparency
   logs since HTTPS certs were issued; once Funnel is on, the name also resolves publicly and
   will be scanned. Every route except the static login page/assets must reject requests without
   an identity header or a valid session (D-D), and registration must reject any request carrying
   `Tailscale-Funnel-Request` or lacking an allowed `Tailscale-User-Login` (D-E).
2. DoS on tailscaled. The per-connection `http.Server` in tailscaled sets no
   `ReadHeaderTimeout`/`ReadTimeout`/`IdleTimeout` (`ipn/ipnlocal/serve.go:648-668`), and the
   backend transport has no `ResponseHeaderTimeout` (`:1001-1018`). Slowloris-style load lands
   on tailscaled, outside soos-remote's control; only Tailscale's undisclosed relay limits apply.
   A known remote CPU DoS (malformed request target looping forever in `getServeHandler`) is
   fixed in this version (`:833-842` comment: "a remote DoS via serve, or via funnel from the
   internet").
3. DoS on soos-remote. Request bodies are streamed through, so a slow or huge body reaches the
   Unix socket. soos-remote must keep explicit bounds: header and body size caps, a per-request
   read deadline, a bounded number of concurrent connections and SSE streams, bounded
   outstanding WebAuthn challenges (count + expiry), bounded sessions (count + TTL), and global
   failure rate limits on login/unlock/registration (D-G). Rate limits keyed on
   `X-Forwarded-For` are hints only (section 2.3).
4. No client IP trust. Do not use `X-Forwarded-For`, `X-Forwarded-Host` or `Host` for any
   authorization decision; only `Tailscale-User-Login` (tailnet path) and verified passkey
   assertions / session tokens (both paths) authorize.
5. SSE through Funnel. Go `ReverseProxy` flushes `text/event-stream` responses immediately
   (`reverseproxy.go:669-673`), so `/api/events` works through Funnel; it must require a session
   on the Funnel path (D-D) and count towards the stream bound.
6. Session cookies. Use `Secure; HttpOnly; SameSite=Strict; Path=/`, host-only (no `Domain`),
   ideally with the `__Host-` prefix: `ts.net` being a public suffix prevents other tailnets from
   setting cookies for this host, and a host-only cookie is not shared with sibling nodes.
   Cookies are not port-isolated, so a session cookie set on 443 is also sent to 8443 on the same
   host (same backend here, so harmless).
7. Logging. tailscaled itself logs ingress source addresses on errors (`handleIngress` logf
   lines); soos-remote must not add identities, credential ids, challenges or tokens to its own
   logs (D-H).
8. Operational. Deployment (A) (Funnel on 443) makes the existing tailnet URL public; deployment
   (B) (Funnel on 8443) keeps 443 tailnet-only and is reversible per port. Both are owner actions
   (D-I); the code must be correct for either.

## 5. Summary of answers

1. Yes, `tailscale funnel --bg unix:<path>` proxies to a Unix socket exactly like serve (same
   code path). Ports 443/8443/10000 (from the node's `funnel-ports` capability). A given port is
   either serve-only or serve+funnel; serve 443 + funnel 8443 can coexist. With Funnel on 443,
   tailnet clients connecting via MagicDNS still get `Tailscale-User-Login`.
2. tailscaled deletes client copies of `Tailscale-User-Login`, `-User-Name`, `-User-Profile-Pic`,
   `Tailscale-Funnel-Request`, `Tailscale-Headers-Info`, `Tailscale-App-Capabilities`, and (via
   Go `ReverseProxy`) `Forwarded`, `X-Forwarded-For/Host/Proto`; for Funnel it sets only
   `Tailscale-Funnel-Request: ?1` plus the forwarding headers; for tailnet users it sets the
   identity headers. `X-Forwarded-Host` mirrors the client `Host`.
3. Prerequisites (MagicDNS, HTTPS, `funnel` nodeAttr, funnel-ports cap) are already satisfied on
   this tailnet; free on all plans; undisclosed bandwidth limits; DNS can take 10 minutes.
4. Treat the Funnel path as hostile internet: explicit bounds everywhere, no IP or Host trust,
   identity only from `Tailscale-User-Login` or a verified passkey.
