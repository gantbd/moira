# Moira peer-effect process qualification

This isolated workspace composes Moira's durable selected-effect saga with the
exact standalone Aegis, Agora, and Vellum owners. It deliberately discards the
accepted Agora response and Vellum edit replies in provider order, reopens
Moira's SQLite owners between each boundary, and proves that recovery uses the
same retained Agora request and Vellum's historical lookup rather than issuing
either effect again.

The qualification depends only on public peer clients and the deployment-owned
Moira adapters. Peer storage, authentication, authority, mutation semantics,
and shutdown remain in their owning processes.
