# GOG native chat feasibility

## Decision

Native chat is not enabled. The required protocol could not be validated safely and completely.

## Evidence checked

- The authenticated `chat.gog.com` REST surface used by current Ludomere friends and invitation reads.
- GOG's public Galaxy integrations Python API, including its social, presence, and notification contracts.
- Current maintained open-source GOG clients used by the protocol audit.

The validated REST surface exposes friend and invitation metadata, but no complete contract for conversation enumeration, paginated history, message sending, receiving, deduplication, or read state. The Galaxy integrations API exposes cross-platform presence/import notifications, not a native GOG one-to-one chat transport suitable for Ludomere. No maintained client source established the missing contracts.

## Failed feasibility gates

- Conversation enumeration and stable conversation identifiers.
- Paginated history and stable message identifiers.
- One-to-one text sending with understood retry behavior.
- Push or bounded-poll receipt with reconnection and token renewal.
- Read-state behavior, rate limits, and offline reconciliation.

Experimental chat code, dependencies, and UI were therefore not added. There is no embedded-web fallback. Reconsideration requires sanitized fixtures plus live receive/send validation for every gate above.
