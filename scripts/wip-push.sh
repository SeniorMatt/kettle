#!/bin/sh
# Push a snapshot of the working tree (committed or not) to the wip-alpha branch, so GitHub checks it.
set -e
idx=$(mktemp -u); export GIT_INDEX_FILE="$idx"
git read-tree HEAD && git add -A
tree=$(git write-tree); rm -f "$idx"; unset GIT_INDEX_FILE
name=$(git log -1 --format=%an); mail=$(git log -1 --format=%ae)
c=$(GIT_AUTHOR_NAME="$name" GIT_AUTHOR_EMAIL="$mail" GIT_COMMITTER_NAME="$name" GIT_COMMITTER_EMAIL="$mail" \
    git commit-tree "$tree" -p HEAD -m "Work in progress")
git push -q -f origin "$c:refs/heads/${1:-wip-alpha5}"
echo "$c"
