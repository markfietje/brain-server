# Visible mixing beats pretend isolation

*2026-09-11. Recall responses now carry `included_global`. This post
explains why a visible flag won over a bigger architectural claim.*

Single-database multi-tenancy has a standard dishonesty. The vendor says
tenants are isolated. The implementation is a `WHERE` clause. One missed
predicate, one admin query, one rescue leg that pulls the shared pool
into a scoped query, and isolation was a label all along.

This server runs a shared pool with a domain shim by design, and the
rescue leg is deliberate: a domain query with thin results borrows from
the global corpus rather than returning nothing. The old behavior mixed
silently. The caller saw hits and could not tell which pool they came
from. That is the exact shape that becomes a cross-tenant incident in a
shared deployment.

`included_global` makes the mixing explicit on every response. A domain
query that borrowed global rows says so. Consumers can filter, auditors
can count, and a deployment that must not mix can refuse any response
with the flag set. True storage isolation remains a separate deployment
mode (`BRAIN_MULTI_DB`), named wherever the flag is documented.

The principle generalizes. A boundary you cannot enforce should be a
field you always emit. Visibility is not isolation, but silent mixing is
how isolation claims die.
