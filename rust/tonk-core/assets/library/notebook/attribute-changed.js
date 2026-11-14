// The routing context can be stamped after connect: mount when it lands.
(self, name) => {
  if (name === 'with' && self.isConnected && !self.__notebook) self.start();
}
