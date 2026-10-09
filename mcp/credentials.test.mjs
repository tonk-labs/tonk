import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createCredentials} from './credentials.mjs';
test('expired access renews, rotates credentials and preserves the fixed authorization lifetime',()=>{
 let now=0;const credentials=createCredentials({now:()=>now,lifetime:100,maxLifetime:1000,capacity:1});
 const value={tenant:'one'},first=credentials.issue(value);now=101;
 assert.equal(credentials.get(first.token),undefined);assert.throws(()=>credentials.issue({tenant:'two'}));
 const second=credentials.renew(first.refreshToken);assert.equal(credentials.get(second.token),value);
 assert.notEqual(second.token,first.token);assert.notEqual(second.refreshToken,first.refreshToken);
 assert.equal(second.refreshExpiresAt,1000);now=1000;
 assert.equal(credentials.renew(second.refreshToken),undefined);assert.equal(credentials.size(),0);
});
test('refresh replay and explicit revocation revoke successors and notify stream owners',()=>{
 let revoked=0;const credentials=createCredentials({onRevoke:()=>revoked++});
 const first=credentials.issue({}),second=credentials.renew(first.refreshToken);
 assert.equal(credentials.renew(first.refreshToken),undefined);
 assert.equal(credentials.get(second.token),undefined);assert.equal(credentials.renew(second.refreshToken),undefined);assert.equal(revoked,1);
 const third=credentials.issue({}),fourth=credentials.renew(third.refreshToken);
 credentials.revoke(third.token);assert.equal(credentials.get(fourth.token),undefined);assert.equal(revoked,2);
});
