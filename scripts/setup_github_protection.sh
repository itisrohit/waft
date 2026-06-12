#!/bin/bash
# Script to automate setting up main branch protection using GitHub CLI.
# Enforces:
# - Required PR reviews (minimum 1 approval)
# - Required status checks (all CI platforms and security audit must pass)
# - Strict matching (branch must be up-to-date before merge)
# - Enforcement on administrators

OWNER="itisrohit"
REPO="waft"
BRANCH="main"

echo "Checking github.com/itisrohit/waft..."

# Check if authenticated
if ! gh auth status &>/dev/null; then
    echo "❌ Error: Not logged in to gh CLI. Please run 'gh auth login' first."
    exit 1
fi

echo "Setting branch protection rules on '${BRANCH}'..."

gh api \
  --method PUT \
  -H "Accept: application/vnd.github+json" \
  "/repos/${OWNER}/${REPO}/branches/${BRANCH}/protection" \
  --input - <<EOF
{
  "required_status_checks": {
    "strict": true,
    "contexts": [
      "Test and Lint (ubuntu-latest)",
      "Test and Lint (macos-latest)",
      "Test and Lint (windows-latest)",
      "Security Audit"
    ]
  },
  "enforce_admins": true,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": false,
    "required_approving_review_count": 1
  },
  "restrictions": null
}
EOF

if [ $? -eq 0 ]; then
    echo "✅ Branch protection configured successfully!"
else
    echo "❌ Failed to set branch protection. Make sure the branch '${BRANCH}' has been pushed to the remote first."
fi
