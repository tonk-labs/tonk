# EditableTitle Tests

## Required Test Coverage

- Display current title from store
- Enter edit mode on click
- Save on Enter key
- Cancel on Escape key
- Revert to "Untitled" when empty after trim
- Truncate titles over 100 characters
- Trim whitespace on save
- Auto-focus and select text in edit mode
- Keyboard navigation support

## Setup Required

Install Vitest and React Testing Library:

```bash
bun add -d vitest @testing-library/react @testing-library/user-event jsdom
```

Add test script to package.json:

```json
"test": "vitest"
```
