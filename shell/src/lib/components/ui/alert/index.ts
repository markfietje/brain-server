import Root from "./alert.svelte";
import { alertVariants, type AlertVariant } from "./alert.js";
import Action from "./alert-action.svelte";
import Description from "./alert-description.svelte";
import Title from "./alert-title.svelte";

export {
	Root,
	alertVariants,
	type AlertVariant,
	Description,
	Title,
	Action,
	//
	Root as Alert,
	Description as AlertDescription,
	Title as AlertTitle,
	Action as AlertAction,
};
