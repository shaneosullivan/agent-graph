# Agent Graph

When multiple agents are interoperating, it can be difficult to understand their relationship to each other:

- What session spawned what agents?
- What session spawned other sessions?
- Is a session waiting on work from another session?
- What is the total amount of work remaining from other sessions before this session can continue?
- What work internally in this session is left to do, concisely explained?
- What messages did one session to another session, and what response did it receive?

This project aims to solve this problem in an implementation agnostic manner.
It should work with open standards to provide an interface for any
AI coding provider to continually have each active session update local JSON files, which a simple standalone program reads to show the state. Everything stays on the local machine for full privacy, unless you choose to share a log on the Agent Graph site, which gives a link (optionally password protected, and kept while the log is in use) that can be opened anywhere, including a phone.
