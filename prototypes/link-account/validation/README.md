# Implementation source fixtures

These screenshots render the checked-in styles.css, account.css, registration
markup, and activate.html. Registration DOM is placed in its waiting state by
a fixture script; this is not authenticated runtime evidence.

Checked in isolated Chrome:
- 1200x900 light: 360px shell, 25px radius, 48px actions.
- 320x480 dark: no horizontal overflow, readable waiting state.
- 320x260: body scrolls, footer remains within the viewport.
- Activation: action follows terms, errors remain visible with confirmation hidden.
- Hub notice: display:none at /settings/link; display:flex at / for a pending account.

Passkeys, email delivery, cross-tab continuation and deployed rendering are not
proven by these fixtures. Existing integration coverage was extended separately.
