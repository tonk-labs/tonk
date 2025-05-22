"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.getProjectRoot = getProjectRoot;
const path_1 = __importDefault(require("path"));
/**
 * Get the project root directory regardless of where the code is running from
 * @returns The absolute path to the project root
 */
function getProjectRoot() {
    // When running from dist, we need to go up one level
    const isRunningFromDist = __dirname.includes("dist") || __dirname.includes("src");
    return isRunningFromDist
        ? path_1.default.resolve(__dirname, "..")
        : path_1.default.resolve(__dirname);
}
