// Run in an isolated loopback browser; creates a disposable fixture, never a signed-in space.
async () => {
 if((location.hostname !== '127.0.0.1' || location.protocol !== 'http:') || !navigator.serviceWorker.controller) throw Error('Isolated worker not ready');
 const call = async (path,body,method='POST') => {
   const r=await fetch(path,{method,headers:{'Content-Type': typeof body==='string'?'text/plain':'application/json'},body:typeof body==='string'?body:JSON.stringify(body)});
   const text=await r.text(); if(!r.ok) throw Error(`${r.status}: ${text.slice(0,1500)}`); return JSON.parse(text);
 };
 const space=await call('/api/repository/Thin%20query%20fixture',{branch:{main:{}}},'PUT');
 const endpoint='/api/repository/'+encodeURIComponent(space.subject)+'/branch/main/evaluate';
 const source=`concept!: &person
  with:
    name: {description: Test fixture field, cardinality: one, the: test.person/name, as: text}
concept!: &issue
  with:
    title: {description: Test fixture field, cardinality: one, the: test.issue/title, as: text}
    assignee: {description: Test fixture field, cardinality: one, the: test.issue/assignee, as: entity}
    status: {description: Test fixture field, cardinality: one, the: test.issue/status, as: text}
    body: {description: Test fixture field, cardinality: one, the: test.issue/body, as: text}
person!:
  this: id:jack
  name: Jack Douglas
person!:
  this: id:jill
  name: Jill
issue!:
  this: id:a
  title: Assigned active
  assignee: id:jack
  status: In progress
  body: '${'x'.repeat(120000)}'
issue!:
  this: id:b
  title: Assigned done
  assignee: id:jack
  status: Done
  body: "done"
issue!:
  this: id:c
  title: Someone else
  assignee: id:jill
  status: In progress
  body: "other"
`;
 await call(endpoint,source);
 const document='person:\n  this: ?person\n  name: "Jack Douglas"\nissue:\n  this: ?issue\n  assignee: ?person\n  title: ?title\n  status: "In progress"\n';
 const response=await call(endpoint+'?transact=false',document);
 const assert = (ok, message) => { if (!ok) throw Error(message); };
 const read = async document => {
   const result = await call(endpoint+'?transact=false', document);
   assert(result.commits.claims === 0, 'Query committed claims');
   assert(JSON.stringify(result.revision_before) === JSON.stringify(result.revision_after), 'Query changed revision');
   return result;
 };
 assert(JSON.stringify(response).length > 100000, 'Fixture must exceed the old tool result limit');
 const people = await read('test.person:\n  this: ?person\n  name: "Jack Douglas"\n');
 const person = people.matches_after[0].results;
 assert(person.length === 1 && person[0].this === 'id:jack', 'Person resolution mismatch');
 const query = `test.issue:\n  this: ?issue\n  assignee: ${person[0].this}\n  title: ?title\n  status: "In progress"\n`;
 const focused = await read(query);
 const rows = focused.matches_after[0].results;
 assert(rows.length === 1 && rows[0].this === 'id:a', 'Assigned active filter mismatch');
 assert(rows[0].fields.title === 'Assigned active' && !('body' in rows[0].fields), 'Projection mismatch');
 assert(JSON.stringify(focused).length < 10000, 'Focused query response is too large');
 const empty = await read(query.replace(person[0].this, 'id:nobody'));
 assert(empty.matches_after[0].results.length === 0, 'Unmatched exact identity must return zero');
 const noPerson = await read('test.person:\n  this: ?person\n  name: "Nobody"\n');
 assert(noPerson.matches_after[0].results.length === 0, 'Missing person must stop issue lookup');
 return {passed: true, workerUnchanged: true, conceptResponseBytes: JSON.stringify(response).length,
   focusedResponseBytes: JSON.stringify(focused).length, matchedIDs: rows.map(row => row.this),
   emptyIdentityMatches: 0, commits: 0, query};
}
