// Claim the mount now and run it a microtask later, if still connected and
// not mounted by then. Claimed at once, so the two ways in (`connected`
// and a `with` arriving) cannot both mount.
(self) => {
  if (self.__mounting) return;
  self.__mounting = true;
  queueMicrotask(() => {
    self.__mounting = false;
    if (self.isConnected && !self.__notebook) self.mount();
  });
}
