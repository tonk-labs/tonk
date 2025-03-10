export interface ProjectInstructions {
  global: {recipe: string};
  components: {recipe: string};
  modules: {recipe: string};
  stores: {recipe: string};
  views: {recipe: string};
}

export class ProjectInstructions {
  global = {recipe: 'RECIPE.md'};
  components = {recipe: 'src/components/RECIPE.md'};
  modules = {recipe: 'src/modules/RECIPE.md'};
  stores = {recipe: 'src/stores/RECIPE.md'};
  views = {recipe: 'src/views/RECIPE.md'};
}
