import assert from "node:assert/strict";
import { test } from "node:test";

import {
    ANSWER,
    answered,
    bringOver,
    handOverThrough,
    leftBehind,
    remember,
    said,
    shouldOffer,
} from "../assets/migration.mjs";

// A browser's storage, or one that refuses every read and write.
const storage = (kept = {}) => ({
    getItem: key => kept[key] ?? null,
    setItem: (key, value) => void (kept[key] = value),
    kept,
});
const refusing = {
    getItem: () => {
        throw new Error("SecurityError");
    },
    setItem: () => {
        throw new Error("SecurityError");
    },
};
const databases = (...names) => ({ databases: async () => names.map(name => ({ name, version: 1 })) });

test("what an earlier release kept is told by the databases only it made", () => {
    assert.equal(leftBehind([{ name: "dialog.credential" }, { name: "did:key:zProfile" }]), true);
    assert.equal(leftBehind([{ name: "tonk" }]), true, "a registry profile from before profiles were kept by DID");
    assert.equal(leftBehind([{ name: "did:key:zSomething" }]), false);
    assert.equal(leftBehind(undefined), false);
});

test("the offer is made where an earlier release left something and nothing was answered", async () => {
    assert.equal(await shouldOffer({ indexedDB: databases("tonk"), storage: storage() }), true);
    assert.equal(await shouldOffer({ indexedDB: databases("did:key:zOther"), storage: storage() }), false);
});

test("an answered offer is not made again", async () => {
    for (const answer of ["declined", "brought"]) {
        const indexedDB = databases("tonk");
        assert.equal(await shouldOffer({ indexedDB, storage: storage({ [ANSWER]: answer }) }), false, answer);
    }
});

test("storage that cannot be read reads as no answer, and the offer is made", async () => {
    assert.equal(answered(refusing), null);
    assert.doesNotThrow(() => remember(refusing, "declined"));
    assert.equal(await shouldOffer({ indexedDB: databases("tonk"), storage: refusing }), true);
});

test("a browser that cannot list its databases is not offered anything", async () => {
    assert.equal(await shouldOffer({ indexedDB: {}, storage: storage() }), false);
    const failing = { databases: async () => Promise.reject(new Error("no")) };
    assert.equal(await shouldOffer({ indexedDB: failing, storage: storage() }), false);
});

test("the spaces read are handed to the profile's worker with what it answers", async () => {
    const posted = [];
    const relay = (message, transfer) => {
        posted.push({ message, transfer });
        const port = transfer[0];
        port.postMessage({ made: message.spaces.map(({ name }) => ({ name, space: `did:key:z${name}` })), failed: [] });
    };
    const kept = [{ subject: "did:key:zOld", name: "Notes", content: new Uint8Array([1, 2]), revision: new Uint8Array([3]) }];
    const outcome = await bringOver({ load: async () => kept, handOver: handOverThrough(relay) });
    assert.deepEqual(outcome, { made: [{ name: "Notes", space: "did:key:zNotes" }], failed: [] });
    assert.equal(posted.length, 1);
    const [{ message, transfer }] = posted;
    assert.equal(message.type, "migrate");
    assert.deepEqual([...new Uint8Array(message.spaces[0].content)], [1, 2]);
    assert.deepEqual([...new Uint8Array(message.spaces[0].revision)], [3]);
    assert.equal(transfer.length, 3, "the reply port and each space's bytes are transferred");
    assert.equal(message.spaces[0].subject, undefined, "the earlier space's DID stays here: the copy is a new space");
});

test("nothing kept on this device alone hands nothing over", async () => {
    let handed = false;
    const outcome = await bringOver({ load: async () => [], handOver: async () => (handed = true) });
    assert.deepEqual(outcome, { made: [], failed: [] });
    assert.equal(handed, false);
});

test("a profile that never answers is reported, not waited on", async () => {
    await assert.rejects(handOverThrough(() => {}, { timeout: 10 })([{ name: "a", content: new Uint8Array(), revision: new Uint8Array() }]), /did not answer/);
});

test("what came of bringing spaces over is said plainly", () => {
    assert.equal(said({ made: [{ name: "a" }], failed: [] }), "Brought over 1 space. You'll find it in your hub.");
    assert.equal(said({ made: [{ name: "a" }, { name: "b" }], failed: [] }), "Brought over 2 spaces. You'll find them in your hub.");
    assert.match(said({ made: [], failed: [] }), /Spaces that sync come back when you sign in/);
    assert.match(said({ made: [], failed: [{ name: "Old notes" }] }), /Couldn't bring over Old notes\. They are still here/);
});
