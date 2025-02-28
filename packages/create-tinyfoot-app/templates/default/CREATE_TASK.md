# How to create a task
Creating a task requires imagining yourself as a grand code architect. It also requires you understand the projectSchema of this project.

Your objectives and rules:
  1.	Collect Requirements Through Dialogue
    -	Engage in a conversation with the user to understand the feature they want.
    -	If needed, ask follow-up questions to clarify any ambiguities or missing details.
    -	Do not produce tasks if you are unsure about any requirements. Instead, keep asking questions until you have enough clarity.
	
  2.	Translate Requirements into Tasks
    -	Once you have enough information, break down the feature request into discrete tasks.
    -	Each task must conform to exactly one of the schemas found in projectSchema.ts.
    -	You can have multiple tasks, but each must be focused on one specific schema scope (e.g., a component schema, a store schema, etc.).
	
  3.	Task Format
    -	Each task must be output as a single JSON in the tasks folder and named .tinyfoot/tasks/{n}.json, where {n} is a counter given whatever is the latest task.
    -	Do not nest multiple tasks in the same file. Each file will contain exactly one JSON object representing one task.
    -	Follow this general structure (pseudo-example; adjust to your actual schema types and fields):
  
  4. Integration task
    - There should always be one last integration task, which simply includes instructions about how all the tasks refer to each other.
    - This task should be named .tinyfoot/tasks/check.json 
    


## Task JSON Format 
{
  schemaType: string // please refer to the ProjectInstructions interface in projectSchema.ts
  description: string // a description suitable enough in detail to understand what to implement and how
  details: object // JSON structure of any details you believe is relevant for the implementation

  // because each task is restricted to a schema, we can make sure the path always includes the schemaName as folder (e.g. src/components for component task) 
  relevantFiles: [{
    // A task can be a greenfield task, which means you need to create completely new files
    // A task can also be a brownfield task which means you must update existing files
    // Delete is self explanatory — this file should no longer be used
    type: 'GREENFIELD' | 'BROWNFIELD' | 'DELETE',
    path: string
  }],
  extraInstructions: `src/${schemaType}/RECIPE.md`,
  completed: boolean // default to false until the user explicitly says it's good
}

## Task JSON Examples
  ```
  {
  "schemaType": "hook",
  "description": "Create a hook to fetch Google Calendar events for the user's primary calendar for the current day.",
  "details": {
    "fields": [
      "events",
      "isLoading",
      "error"
    ],
    "initialState": {
      "events": [],
      "isLoading": false,
      "error": null
    },
    "actions": [
      "setLoading",
      "setEvents",
      "setError"
    ],
    "notes": "Use a simple structure that other hooks/components can import to read from calendar state."
  },
  "relevantFiles": [
    {
      "type": "GREENFIELD",
      "path": "src/hooks/fetchCalendarData.ts"
    }
  ],
  "extraInstructions": [
    {
      "path": "src/hooks/RECIPE.md"
    }
  ],
  "completed": false
}
  ```

## Conversation Flow Examples

User>

I want my pomodor app to use calendar data

LLM>

Certainly! Before creating the task, I want to clarify a few things:
	1. Where would you like to fetch calendar data from?
  2. Is there specific type of calendar data or any specific calendar?

User>

I want to fetch it from google calendar. Just use the primary calendar and find all the events for that day and include them in a list below the pomodoro timer.


[... and so on]

## Additional Information

  5.	Best Practices / Recommendations
    - Make sure each task has enough detail so that another developer (or another LLM) can implement it without asking for more information.
    - Tasks should be modular and discrete enough to be testable with regular unit tests (unless they are React components or UI bits then use React best practices).


## LLM Conversation Tips

1. Always begin by understanding the users request.
2. ALWAYS Ask clarifying questions if the request is incomplete or vague.
3. Once you have enough information, produce the tasks as described.
4. Clarify that the task was created correctly.
5. If the task was created correctly, move on to the next task.


# Finally

Make sure to load the tinyfoot.config.json for global context every time!!!