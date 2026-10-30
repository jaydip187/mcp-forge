import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

/** The "Require approval" switch of the tool editors. */
export function ApprovalSwitch({ id, checked, onChange }: { id: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="flex items-start gap-2">
      <Switch id={id} checked={checked} onCheckedChange={onChange} className="mt-0.5" />
      <div className="grid gap-0.5">
        <Label htmlFor={id}>Require approval</Label>
        <p className="text-xs text-muted-foreground">
          Agent calls wait until a person approves them, in mcp-forge under Approvals or in the agent's app when it can ask.
          Test console runs don't wait.
        </p>
      </div>
    </div>
  );
}
