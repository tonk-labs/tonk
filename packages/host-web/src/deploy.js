const path = require("path")
const fs = require("fs")
const { create, change } = require("@automerge/automerge")

const distDir = "dist"
const files = fs.readdirSync(distDir, { withFileTypes: true })