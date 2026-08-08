# Policy graph worker boundary

Minutes builds People, relationship, and commitment projections from one
ordered, policy-filtered corpus snapshot. Restricted meetings never enter that
snapshot. The parent publishes a worker result only after the correction and
corpus journal proves that the snapshot stayed current through the operation.

On a trusted, distributed Minutes app running macOS 12 or newer, the projection
runs in the embedded `com.useminutes.graph-worker` XPC service. Before sending
any meeting bytes, both peers install code-signing requirements and complete a
content-free handshake. The parent requires the exact service CodeDirectory
hash sealed into that app; the service requires a Minutes desktop identifier
from Team `63TMLKT8HN`. The App-Sandbox-only service also applies process,
address-space, input/output, and wall-clock limits. Each service process admits
one authenticated projection, sends one terminal response, and exits. The
parent observes that exit before returning either success or failure; local
reader and budget failures first send a content-free terminal abort. Each
service process generates a random nonce. The content-free `begin` reply binds
the parent to that nonce, every later request must carry it, and every reply
echoes it. A terminal reply and the connection-end event may reach their shared
serial callback queue in either order; settlement requires both and the
terminal reply must still match the bound nonce. Any earlier interruption
immediately poisons the transport, and a stale abort cannot claim or terminate
a relaunched helper because only `begin` may claim an awaiting process. Requests
from one parent are serialized under the same absolute deadline, with poison
rechecked after lock admission. A competing authenticated parent receives a
nonterminal `busy` response without any private bytes crossing, and teardown of
that rejected peer cannot terminate the active owner. If exit cannot be proven
within the original deadline, authenticated graph transport likewise fails
closed until the app restarts. A later projection therefore receives a fresh
process and cannot raise or extend the prior process's limits.

macOS 11 cannot install the low-level XPC peer code-signing requirement. Source
builds, ad-hoc builds, and standalone CLI use likewise cannot prove the shipped
app/XPC identity. Those modes keep the existing default-user graph experience
by projecting the already filtered normal corpus in process. This is an
explicit compatibility fallback, not an atomic helper-isolation guarantee.
Trusted apps on supported macOS versions fail closed if their embedded XPC
service or sealed identity is missing.

The long-term graph privacy work is tracked in
[GitHub issue #513](https://github.com/silverstein/minutes/issues/513).
