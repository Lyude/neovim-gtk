#!/bin/bash

summ() {
	echo "$@" >> $GITHUB_STEP_SUMMARY
}

cd $GITHUB_WORKSPACE
if [[ "$(git status -s)" != "" ]]; then
	summ "The repository wasn't clean after running \`cargo test\`."
	summ ""
	summ "\`git status\` shows the following:"
	summ ""
	summ '```'
	git status >> $GITHUB_STEP_SUMMARY
	summ '```'
	summ ""
	summ "My guess is that you did one of these things:"
	summ ""
	summ "- Changed something in \`.github/workflows/*\` and forgot to update \`.gitignore\`"
	summ "- Changed \`Cargo.toml\` and forgot to stage the changes to \`Cargo.lock\`"
	exit 1
fi
