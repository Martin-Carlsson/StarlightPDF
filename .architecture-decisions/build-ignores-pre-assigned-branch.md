# Build ignores the pre-assigned branch

Cloud sessions start on a branch with a random name such as `claude/nice-hamilton-6ww6sb`. Build creates and pushes its own named branch and never pushes the pre-assigned one, because otherwise cloud builds keep the random names.
