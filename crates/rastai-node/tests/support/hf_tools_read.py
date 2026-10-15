"""Reads a hub cache the node wrote, the way the other Hugging Face tools do, offline (contracts 8.1; layout 7.9).

The node's test runs this with HF_HUB_OFFLINE=1, so huggingface_hub may only read what is on disk. Exit 0 when every
read finds the repository at the commit, else exit 1 with the reason on stderr.
"""

import sys

from huggingface_hub import hf_hub_download, scan_cache_dir, snapshot_download

USAGE = "usage: hf_tools_read.py <hub cache> <repo id> <commit> <config file name>"


def refuse(reason):
    print(reason, file=sys.stderr)
    sys.exit(1)


def main(arguments):
    if len(arguments) != 4:
        refuse(USAGE)
    cache_dir, repo_id, commit, config_name = arguments
    snapshot = snapshot_download(repo_id, cache_dir=cache_dir, local_files_only=True)
    if not snapshot.endswith(commit):
        refuse(f"snapshot_download gave {snapshot}, not the commit {commit}")
    config_path = hf_hub_download(repo_id, config_name, cache_dir=cache_dir, local_files_only=True)
    if not config_path.startswith(snapshot):
        refuse(f"hf_hub_download gave {config_path}, outside the snapshot")
    repos = [repo for repo in scan_cache_dir(cache_dir).repos if repo.repo_id == repo_id]
    if len(repos) != 1:
        refuse(f"scan_cache_dir found {len(repos)} copies of {repo_id}, not one")
    revisions = {revision.commit_hash for revision in repos[0].revisions}
    if commit not in revisions:
        refuse(f"scan_cache_dir lists {sorted(revisions)}, not {commit}")


if __name__ == "__main__":
    main(sys.argv[1:])
