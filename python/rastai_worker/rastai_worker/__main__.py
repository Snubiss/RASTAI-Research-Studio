"""`python -m rastai_worker`: the process every Python job runs in (contracts 3.1; python layout 7.1).

A thin main: parse the command line, then run the mode. The exit code is the job's (exit_codes.py).
"""

import sys

from rastai_worker.arguments import MODE_PLUGIN, USAGE, parse_arguments
from rastai_worker.diagnostics import error_line, print_diagnostic_line
from rastai_worker.errors import WorkerError
from rastai_worker.exit_codes import EXIT_BAD_LAUNCH
from rastai_worker.launch import run_job_process
from rastai_worker.plugin_module import run_plugin_process


def main(argv: list[str]) -> int:
    try:
        arguments = parse_arguments(argv)
    except WorkerError as error:
        print(USAGE, file=sys.stderr)
        print_diagnostic_line(error_line(error))
        return EXIT_BAD_LAUNCH
    if arguments.mode == MODE_PLUGIN:
        return run_plugin_process(arguments.plugin_id, arguments.job_file)
    return run_job_process(arguments.job_file)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
