(self, name, before, after) => {
  if (name === 'start' && after && self.__deck?.started) self.goTo(after);
}
