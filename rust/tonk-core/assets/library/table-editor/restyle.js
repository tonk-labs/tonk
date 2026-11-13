// Adopt the element's direct `<style>` children into the shadow root.
(self) => {
  const state = self.__table;
  const css = state.core.userStyles(self);
  if (state.user.textContent !== css) state.user.textContent = css;
}
