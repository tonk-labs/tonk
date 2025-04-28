import { todoStore } from "./stores/todoStore";

function run() {
  const state = todoStore.getState();
  let counter = 0;
  setInterval(() => {
    // This is just as an example of how to use the zustand stores in Node
    counter++;
    state.addTodo(`Todo number ${counter}`);
    if (state.todos.length > 1) {
      state.deleteTodo(state.todos[0].id);
    }
  }, 2000);
}

run();
