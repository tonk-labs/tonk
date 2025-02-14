interface ProjectInstructions {
  global: { recipe: string };
  components: { recipe: string };
  hooks: { recipe: string };
  lib: { recipe: string };
  services: { recipe: string };
  stores: { recipe: string };
  views: { recipe: string };
}

class ThisProject implements ProjectInstructions {
  global = { recipe: 'RECIPE.md' };
  components = { recipe: 'src/components/RECIPE.md' };
  hooks = { recipe: 'src/hooks/RECIPE.md' };
  lib = { recipe: 'src/hooks/RECIPE.md' };
  services = { recipe: 'src/services/RECIPE.md' };
  stores = { recipe: 'src/stores/RECIPE.md' };
  views = { recipe: 'src/views/RECIPE.md' };
}
