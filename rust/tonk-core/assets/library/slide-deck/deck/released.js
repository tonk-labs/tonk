(self) => {
  self.disconnected();
  self.__deck = null;
  self.shadowRoot?.replaceChildren();
}
