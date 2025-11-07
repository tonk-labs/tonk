# 8 Jan 2024

## Global

- Basic component system is working
- Urgently need a gameplay loop
  - Even if it's just caring for your animals (could be fun)
  - Start with single animal
    - Animal has hunger
    - Needs to be fed
    - Animal can die of hunger
    - After X time animal provides you with something

- Building needs to become a component so we can move to ECS
  - Put everything on-chain
- Can we use Arancina with Mud?

- Fix inventory slots
  - stackable combine
- Setting the block reference in renderer breaks the islands
  - circular references?
  - this could be fixed by the ECS

## Generator

- Determine how to make building system work with tiles
- Generator useful for 'combining farms' into a single tile
- Can still have Up, Right, Down, Left for tiles
- ABOUT GRID
  - We calculate the grid on the fly but it's deterministic
  - When building a new neighbour we need to propagate cache in 6 dir
  - We need to be able to get neighbour by coordinates
    - How to deal with Hex edges
    - Can store quad is edge
    - Need for tile generator propagation / WFC type of algos
    - Speeds things up a lot

## Notes

### Animals

ANIMALS ARE YOUR PETS

- If you’re not around they get sad from missing you
- If you care for them they’re happy and playful
- if you make a loud sound the goat falls over
- use shit of the goat to fertilize farms
- animals are eating trash > what happens while you’re gone?!
- dog barks at you when you sign back into the game

### Other

V1 >

- start game with 50 player slots,
- generate 50 islands
- players get an island with an animal
- keep your animal alive using the wheat farm system

pump water mill grain

building is communicating ecosystems, fragility when you maximize profit you lose

use normal tile grid but just arbitrarily warp the grid towards ‘random’ points keeps neighbour
logic simpler makes it simpler to build on MUD

roadside picknick the affair of passing gods shit happened the big mystery is that the gods actually
really don’t care

- initially the rocks have nearly endless resources
- but once you start polluting a rock, throwing trash around, not cleaning up
- water becomes dirty and plants stop growing and trees stop growing
- a mix of cowboy / astronaut? The wild west because of the frontier, why does shit fly
- music: https://youtu.be/JwmoDJbnJfM?list=TLPQMzExMjIwMjN7g8b4bSEPRQ&t=2176

delay mechanism in oxygen not included (your people become bottleneck in speed, but more people is a
problem)

1. you can find a lot of different items, but not all of them have value to you
2. it is particular what sort of items you need, so you can ask other players for these items
3. this of this like the oxygen people supplying you but social
4. in oxygen not included when you fuck up your system your colonists die, in factorio shit just
   starts going slow, there’s more room for fucking up

- VFX for shearing lamb (clouds, plush flying out, buzz sound)
- VFX for harvesting wheat (clouds, wheat flying out)

### Generator

Even the normal building system already uses constraints (“sides”, “up”, “down” etc)

# 27 Dec 2023

## One shot + want

- Clouds ✅- Detect Islands Create island class, remap once block added to island Setup island
  center Setup island radius
  - Use island for camera center
  - Distance based fog
- EBlock neighbour cache
  - recalculate neighbours after building block

## Big chunks

- WFC like tiling system
- Converting buildings to simEntities

## Dunno chunks

- Inventory vs Container
- Invslot?

## Flour

- Set up baking chain water falls down, creating downward architecture wheat falls down, using
  gravity to transport grain mill elevators

wheat > mill > flour > flour + milk + egg = pancake flour + water = bread

_flour dust is explosive_, can level city blocks

engineers are how do we make better machinery exhaust fans pulls out the flour dust europeans used
steel rollers unlike stones > brought to end 2000 years of millstones flour was the original
convenience food

- water-mill needs water next to it
- bakery needs house next/close to it
  - house needs food

- swap around block to become a component ?
  - then buildings can become items

## Sinks for resources > https://youtu.be/nHr6B6IdmS0?list=TLPQMjYxMjIwMjPMi0n4w7divg

## Old

✅ need stackables so can multiply wheat using farm ✅ need to set up player ✅ player is a
simentity ✅ where do we store the player entity ✅ currently in currentplayer

- have to be very careful with serialization because we can’t store ISimEntity by reference
- difference between blocks, entities, players, etc
- they should all become entities

✅ inventory also doesn’t haev reactivity

- same goes for farm entities etc
- need fine grained reactivity for rendering

- why would ECS be better
  - currently we predefine our entity data objects in an array
  - they all have separate render functions in separate files
  - but we check out input on the blocks
  - ideally, we only need to check the ‘blocks’ when we’re doing building
  - when not in buildmode, we want to have full raycasting against entities
  - ECS still necessitates building separate renderers for each item
  -
