"""JobEvents: where a job's events go (layout frontend-and-plugins.md 11.2; python.md 1.5).

Engines and plugin jobs emit `(event_type, dict)` through it and never see the wire types (python layout 1.4 rule 7).
rastai_worker implements it over the run's event log and the live stream; a test keeps the events in lists.
"""


class JobEvents:
    """The base class a job emits durable and ephemeral events through."""

    def emit_durable(self, event_type: str, payload: dict, blob_hashes: list[str] | None = None) -> None:
        """Appends one event to the run's event log. `blob_hashes` names the blobs the event refers to."""
        raise NotImplementedError(f"{type(self).__name__} must implement emit_durable")

    def emit_ephemeral(self, event_type: str, payload: dict) -> None:
        """Sends one live event (progress, a log line, a peek). It may be dropped under backpressure."""
        raise NotImplementedError(f"{type(self).__name__} must implement emit_ephemeral")
