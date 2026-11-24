(self, hlc, value) =>
  hlc === null
    ? value
    : `Tonk-Prose-Version: 1\r\nETag: "${hlc}"\r\nContent-Type: text/markdown\r\n\r\n${value}`
