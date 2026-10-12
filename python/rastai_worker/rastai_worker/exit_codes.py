"""The worker's exit codes (contracts 9.1; python layout 7.2; platform flow 5.4).

The node turns each one into the job's end state (rastai-node `jobs/exit_cause.rs`, which keeps the same numbers):
an exit says how the job ended, and the run's event log says why.
"""

# Finished, and the log's final run.state_changed says completed (also after STOP).
EXIT_OK = 0
# Failed; a diagnostic.reported with the reason is in the log.
EXIT_FAILED = 1
# job.json unreadable, a job kind this worker can't run, or no connection to the node; the reason is the last line
# of stderr.log (RRS-J2001).
EXIT_BAD_LAUNCH = 2
# CANCEL (or SIGINT) honored; no checkpoint.
EXIT_CANCELLED = 3
# The log can't be appended even after the reserve was freed (RRS-S3002, plan §10 row 4).
EXIT_EVENT_LOG_UNWRITABLE = 4
# Another process holds the log's flock; nothing was written to it, the diagnostic is in stderr.log (RRS-J3006).
EXIT_EVENT_LOG_LOCKED = 5
# SIGTERM honored, for every kind, even one with nothing to save (EX_TEMPFAIL: relaunch to resume).
EXIT_PREEMPTED = 75
