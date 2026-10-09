import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';

test('shared browser/embed theme follows OS changes without dropping palette classes',()=>{
  const source=readFileSync(new URL('../rust/tonk-ui/assets/space-theme.js',import.meta.url),'utf8');
  const classes=new Set(['embedding-host']);let changed;
  const classList={add:(...values)=>values.forEach(value=>classes.add(value)),toggle:(value,on)=>on?classes.add(value):classes.delete(value)};
  runInNewContext(source,{document:{documentElement:{classList}},window:{matchMedia:query=>{
    assert.equal(query,'(prefers-color-scheme: dark)');
    return {matches:false,addEventListener:(event,callback)=>{assert.equal(event,'change');changed=callback;}};
  }}});
  assert.deepEqual([...classes].sort(),['embedding-host','wa-light','wa-palette-shoelace','wa-theme-default']);
  changed({matches:true});
  assert.deepEqual([...classes].sort(),['embedding-host','wa-dark','wa-palette-shoelace','wa-theme-default']);
});
