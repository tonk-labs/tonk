import {randomBytes,createHash} from 'node:crypto';
const key=token=>typeof token==='string'&&/^[A-Za-z0-9_-]{43}$/.test(token)?createHash('sha256').update(token).digest('hex'):undefined;
const random=()=>randomBytes(32).toString('base64url');
// Fixed family lifetime; rotation never extends the original authorization.
// Retain bounded spent hashes to revoke a family if a refresh token is replayed.
export function createCredentials({now=Date.now,lifetime=600_000,maxLifetime=8*60*60_000,capacity=64,onRevoke=()=>{},saved=[],encode=value=>value,decode=value=>value}={}){
  const families=new Set(),access=new Map(),refresh=new Map();
  function revoke(family){if(!families.delete(family))return;for(const hash of family.accesses)access.delete(hash);for(const hash of family.refreshes)refresh.delete(hash);onRevoke(family.value);}
  function prune(){for(const family of families)if(family.until<=now())revoke(family);}
  function rotate(family){
    if(family.refreshes.size>=256){revoke(family);return;}
    const token=random(),refreshToken=random();family.access=key(token);family.current=key(refreshToken);
    family.accesses.add(family.access);
    family.expiresAt=Math.min(now()+lifetime,family.until);family.refreshes.add(family.current);
    access.set(family.access,family);refresh.set(family.current,family);
    return {token,refreshToken,expiresAt:family.expiresAt,refreshExpiresAt:family.until};
  }
  if(!Array.isArray(saved)||saved.length>capacity)throw Error('Invalid credential snapshot');
  for(const item of saved){
    const {accesses,refreshes,access:active,current,expiresAt,until}=item;
    const valid=hash=>typeof hash==='string'&&/^[a-f0-9]{64}$/.test(hash);
    if(!Array.isArray(accesses)||!Array.isArray(refreshes)||accesses.length>256||refreshes.length>256||!accesses.every(valid)||!refreshes.every(valid)||!accesses.includes(active)||!refreshes.includes(current)||!Number.isFinite(expiresAt)||!Number.isFinite(until)||expiresAt>until)throw Error('Invalid credential snapshot');
    if(until<=now())continue;
    const family={value:decode(item.value),accesses:new Set(accesses),refreshes:new Set(refreshes),access:active,current,expiresAt,until};
    families.add(family);for(const hash of accesses)access.set(hash,family);for(const hash of refreshes)refresh.set(hash,family);
  }
  return {
    snapshot(){prune();return [...families].map(family=>({...family,value:encode(family.value),accesses:[...family.accesses],refreshes:[...family.refreshes]}));},
    size(){prune();return families.size;},
    issue(value,{until=now()+maxLifetime}={}){prune();if(families.size>=capacity)throw Error('Connection capacity reached');const family={value,until:Math.min(until,now()+maxLifetime),refreshes:new Set(),accesses:new Set()};families.add(family);return rotate(family);},
    get(token){prune();const family=access.get(key(token));return family&&family.access===key(token)&&family.expiresAt>now()?family.value:undefined;},
    renew(token){prune();const hash=key(token),family=refresh.get(hash);if(!family)return;if(family.current!==hash){revoke(family);return;}return rotate(family);},
    revokeHash(hash){const family=access.get(hash);if(family)revoke(family);},
    revoke(token){const family=access.get(key(token));if(family)revoke(family);},
    revokeWhere(predicate){for(const family of families)if(predicate(family.value))revoke(family);},
    close(){for(const family of families)revoke(family);},
  };
}
