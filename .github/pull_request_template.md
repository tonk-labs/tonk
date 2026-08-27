## What changed

<!-- Describe the user or system outcome, not only the implementation. -->

## Verification

<!-- List fresh commands and any relevant browser or CLI evidence. -->

## Storybook impact

Choose one and explain it:

- [ ] User-visible browser or CLI behavior changed. I updated these screen, journey, verification, or triage IDs: <!-- IDs -->
- [ ] This fixes a user-visible bug. The regression test and Storybook now share this ID: <!-- ID -->
- [ ] No user-visible contract changed because: <!-- reason -->

For Storybook changes, run:

```console
python3 docs/storybook/scripts/build.py --check
python3 docs/storybook/scripts/check-links.py docs/storybook
```
