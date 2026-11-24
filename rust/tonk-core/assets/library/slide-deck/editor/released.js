(self) => {
  const s = self.__edit;
  s?.observer?.disconnect();
  clearTimeout(s?.timer);
  self.__edit = null;
  self.shadowRoot?.replaceChildren();
}
