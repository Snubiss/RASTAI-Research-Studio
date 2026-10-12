"""The worker's command line (contracts 3.1 and 15.2; python layout 7.1).

The node starts a job as `python -m rastai_worker run --job-file <run folder>/job.json` and a plugin's module as
`python -m rastai_worker plugin --plugin <plugin_id> --job-file <path>`. The `rank` mode joins with multi-GPU
training and `check` with the node's environment check (python layout 7.11). The parser is a few lines by hand:
argparse exits the process on its own when it doesn't like a line, and a wrong line here must end as a bad launch
with a coded reason the node can show.
"""

from dataclasses import dataclass
from pathlib import Path

from rastai_plugin.catalog.generated import RRS_J2001

from rastai_worker.errors import WorkerError

MODE_RUN = "run"
MODE_PLUGIN = "plugin"
JOB_FILE_OPTION = "--job-file"
PLUGIN_OPTION = "--plugin"
# The options each mode takes; every one is required and takes one value.
MODE_OPTIONS = {
    MODE_RUN: (JOB_FILE_OPTION,),
    MODE_PLUGIN: (PLUGIN_OPTION, JOB_FILE_OPTION),
}
USAGE = (
    "usage: python -m rastai_worker run --job-file <job.json>\n"
    "       python -m rastai_worker plugin --plugin <plugin_id> --job-file <job.json>"
)


@dataclass(frozen=True)
class WorkerArguments:
    """One parsed command line. `plugin_id` is empty in run mode."""

    mode: str
    job_file: Path
    plugin_id: str = ""


def parse_arguments(argv: list[str]) -> WorkerArguments:
    """The mode and its options from `argv` (without the program name). A line that isn't one of the two forms is
    RRS-J2001: the node passed something this worker doesn't understand."""
    if not argv or argv[0] not in MODE_OPTIONS:
        given = argv[0] if argv else "nothing"
        raise usage_error(f"the mode is {given!r}, not one of {', '.join(MODE_OPTIONS)}")
    mode = argv[0]
    values = read_option_values(argv[1:], MODE_OPTIONS[mode])
    return WorkerArguments(mode=mode, job_file=Path(values[JOB_FILE_OPTION]), plugin_id=values.get(PLUGIN_OPTION, ""))


def read_option_values(words: list[str], allowed_options: tuple[str, ...]) -> dict[str, str]:
    """`--option value` pairs, each option once, each one required."""
    values = {}
    for position in range(0, len(words), 2):
        option = words[position]
        if option not in allowed_options:
            raise usage_error(f"{option!r} isn't an option here")
        if option in values:
            raise usage_error(f"{option} is given twice")
        if position + 1 >= len(words) or not words[position + 1]:
            raise usage_error(f"{option} needs a value")
        values[option] = words[position + 1]
    for option in allowed_options:
        if option not in values:
            raise usage_error(f"{option} is missing")
    return values


def usage_error(reason: str) -> WorkerError:
    return WorkerError(RRS_J2001, f"its command line is wrong: {reason}")
