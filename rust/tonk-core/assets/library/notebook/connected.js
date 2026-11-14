// Mount once, a microtask late, and only if still connected.
//
// A renderer moving this element detaches and re-attaches it in one task,
// and a render pass can detach it again before the reaction runs; mounting
// then would build an editor inside an orphan. A re-attach of a mounted
// notebook keeps everything it had.
(self) => {
  if (self.__notebook) {
    self.__notebook.detached = false;
    return;
  }
  self.start();
}
