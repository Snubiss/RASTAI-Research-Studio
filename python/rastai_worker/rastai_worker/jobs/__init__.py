"""The job kinds' entries (python layout 7.1 and 7.4). `dispatch.JOB_ENTRIES` names each one as
`<module>:<function>` and imports its module only when a job of that kind starts, so nothing is imported here.
`train.py` is the first (M1-46); the others land with their build steps (layout 7.11)."""
