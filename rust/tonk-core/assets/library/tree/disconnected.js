// Drop the state, so loads still in flight find it gone and stop.
(self) => {
  self.__tree = null;
}
