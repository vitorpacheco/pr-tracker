export namespace app {

	export interface InstanceStatus {
	    Name: string;
	    Disabled: boolean;
	    Error: string;
	}
	export interface ToolStatus {
	    Name: string;
	    Path: string;
	    Hint: string;
	}
	export interface Diagnostics {
	    Tools: ToolStatus[];
	    Instances: InstanceStatus[];
	}


}

export namespace config {

	export interface Repo {
	    Instance: string;
	    Name: string;
	    Path: string;
	    Remote: string;
	    TrackAll: boolean;
	    MergeMethod: string;
	}
	export interface Instance {
	    Name: string;
	    Provider: string;
	    Host: string;
	    Disabled: boolean;
	    MergeMethod: string;
	    AutoMerge: boolean;
	    DeleteBranch: boolean;
	}
	export interface Config {
	    Language: string;
	    ToolPaths: Record<string, string>;
	    DesktopTerminal: string;
	    RefreshInterval: string;
	    WorktreeDir: string;
	    Terminal: string;
	    DiffTool: string;
	    CloneRoots: string[];
	    Instances: Instance[];
	    Repos: Repo[];
	}


}

export namespace main {

	export interface Item {
	    Data: provider.Item;
	    Key: string;
	    Ref: string;
	    Clone: string;
	    Worktree: string;
	}
	export interface Request {
	    Key: string;
	    Action: string;
	    Body: string;
	    RemoveAfter: boolean;
	    Force: boolean;
	}
	export interface View {
	    Language: string;
	    RefreshSeconds: number;
	    Instances: config.Instance[];
	    Items: Item[];
	    Errors: Record<string, string>;
	    CacheError: string;
	    // Go type: time
	    SyncedAt: any;
	    Revision: number;
	}
	export interface Result {
	    Message: string;
	    Error: string;
	    Dirty: boolean;
	    NeedsClone: boolean;
	    ReloadThread: boolean;
	    View: View;
	}

}

export namespace provider {

	export interface Check {
	    Name: string;
	    State: string;
	    URL: string;
	}
	export interface Comment {
	    Author: string;
	    Body: string;
	    // Go type: time
	    CreatedAt: any;
	    Review: string;
	    Path: string;
	    Line: number;
	}
	export interface Item {
	    Kind: number;
	    Instance: string;
	    Provider: string;
	    Host: string;
	    Repo: string;
	    RepoURL: string;
	    Number: number;
	    Title: string;
	    URL: string;
	    Author: string;
	    Draft: boolean;
	    SourceBranch: string;
	    TargetBranch: string;
	    FromFork: boolean;
	    // Go type: time
	    CreatedAt: any;
	    // Go type: time
	    UpdatedAt: any;
	    Additions: number;
	    Deletions: number;
	    Files: number;
	    Review: string;
	    ApprovedBy: string[];
	    ApprovedByMe: boolean;
	    Conflicts: boolean;
	    CI: string;
	    Checks: Check[];
	    Labels: string[];
	    Assignees: string[];
	    Comments: number;
	    Relations: number;
	}
	export interface MergeOptions {
	    Method: string;
	    Auto: boolean;
	    DeleteBranch: boolean;
	}
	export interface Thread {
	    Body: string;
	    Comments: Comment[];
	}

}
