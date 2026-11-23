package tui

import (
	"fmt"
	"strings"

	"github.com/propheci/botconf/go/describe"
)

// The Environment screen: the variables the program reads itself
// (describe.Description.Env), their values set in the env file.

// envRow opens the Environment screen, on the root screen.
func envRow() row { return row{isEnv: true, label: "Environment"} }

// envRows are the Environment screen's rows: one a variable.
func (a *app) envRows() []row {
	rows := make([]row, len(a.description.Env))
	for index := range a.description.Env {
		rows[index] = row{variable: &a.description.Env[index]}
	}
	return rows
}

func variableTitle(variable *describe.Variable) string {
	if variable.Title != "" {
		return variable.Title
	}
	return variable.Name
}

func (a *app) envShown(r row) shown {
	if r.isEnv {
		set := 0
		for _, variable := range a.description.Env {
			if value, ok := a.env.Get(variable.Name); ok && value != "" {
				set++
			}
		}
		return shown{shownOpens, fmt.Sprintf("%d of %d set", set, len(a.description.Env))}
	}
	value, ok := a.env.Get(r.variable.Name)
	switch {
	case ok && value != "" && r.variable.Secret:
		return shown{shownSet, "•••••• (set)"}
	case ok && value != "":
		return shown{shownSet, value}
	case r.variable.Required:
		return shown{kind: shownMissing}
	default:
		return shown{kind: shownUnset}
	}
}

func (a *app) envDetails(r row) (string, []string) {
	if r.isEnv {
		return "Environment", []string{"The environment variables " + a.description.Title +
			" reads itself, by their names: their values are set in " + a.envName + ".", "Enter: open"}
	}
	variable := r.variable
	var paragraphs []string
	if variable.Description != "" {
		paragraphs = append(paragraphs, strings.Split(variable.Description, "\n\n")...)
	}
	facts := []string{"Variable: " + variable.Name + ", set in " + a.envName}
	if variable.Required {
		facts = append(facts, "Required")
	}
	if variable.Secret {
		facts = append(facts, "Secret: masked here")
	}
	facts = append(facts, "Enter: set its value (empty: unset)")
	return variableTitle(variable), append(paragraphs, strings.Join(facts, "\n"))
}

// openEnv opens the Environment screen.
func (a *app) openEnv() {
	a.screens = append(a.screens, screen{title: "Environment", env: true})
}

// editVariable edits a variable's value in the env file.
func (a *app) editVariable(variable *describe.Variable) {
	value, _ := a.env.Get(variable.Name)
	note := "Empty: unset."
	if variable.Required {
		note = "Required."
	}
	a.mode = editing
	a.edit = edit{
		title:  variable.Name + " in " + a.envName,
		target: envTarget,
		envVar: variable.Name,
		input:  newInput(value),
		secret: variable.Secret,
		note:   note,
	}
}
