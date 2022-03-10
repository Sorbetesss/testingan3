#!/usr/bin/env bash
#
# This script obtains the changelog to be introduced in the new release.

set -eu

function usage() {
    cat <<HELP_USAGE
This script obtains the changelog between the latest release tag and origin/master.

Usage: $0 [-h]

    -h  Print help message.
HELP_USAGE
}

function log_error() {
    echo "Error:" "$@" >&2
    exit 1
}

function log_info() {
    echo -e "[+]" "$@"
}

while getopts "h?" opt; do
    case "$opt" in
        h|\?)
            usage
            exit 0
            ;;
    esac
done

GIT_BIN=$(which git) || log_error 'git is not installed. Please follow https://github.com/git-guides/install-git for instructions'

# Generate the changelog between the provided tag and origin/master.
function generate_changelog() {
    local tag="$1"
    # From the remote origin url, get a link to pull requests.
    remote_link=$($GIT_BIN config --get remote.origin.url | sed 's/\.git/\/pull\//g') || log_error 'Failed to get remote origin url'

    prs=$($GIT_BIN --no-pager log --pretty=format:"%s" "$tag"..origin/master) || log_error 'Failed to obtain commit list'

    log_info "Changelog\n"
    while IFS= read -r line; do
        # Obtain the pr number from each line. The regex should match, as provided by the previous grep.
        if [[ $line =~ "(#"([0-9]+)")"$ ]]; then
            pr_number="${BASH_REMATCH[1]}"
        else
            continue
        fi

        # Generate a valid PR link.
        pr_link="$remote_link$pr_number"
        # Generate the link as markdown.
        pr_md_link=" ([#$pr_number]($pr_link))"
        # Print the changelog line as `- commit-title pr-link`.
        echo "$line" | awk -v var="$pr_md_link" '{NF--; printf "- "; printf; print var}'
    done <<< "$prs"
}

# Get latest release tag.
tag=$($GIT_BIN describe --match "v[0-9]*" --abbrev=0 origin/master) || log_error 'Failed to obtain the latest release tag'
log_info "Latest release tag: $tag"

generate_changelog "$tag"
