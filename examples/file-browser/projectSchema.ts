interface ProjectInstructions {
  global: { recipe: string };
  components: { recipe: string };
  modules: { recipe: string };
  stores: { recipe: string };
  views: { recipe: string };
}

class ThisProject implements ProjectInstructions {
  global = { recipe: 'llms.txt' };
  components = { recipe: 'src/components/llms.txt' };
  modules = { recipe: 'src/modules/llms.txt' };
  stores = { recipe: 'src/stores/llms.txt' };
  views = { recipe: 'src/views/llms.txt' };
}
