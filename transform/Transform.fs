open System
open System.IO
open FSharp.Compiler.Text
open FSharp.Compiler.Tokenization

let wrapFloatToken (tokenText: string) : string =
    // Assuming the function you want to wrap is called "live"
    sprintf "live(%s)" tokenText

let isFloatLiteral (tokenInfo: FSharpTokenInfo) : bool =
    printf ("token name: %s") tokenInfo.TokenName
    // Check if the token is a float literal by its token name
    tokenInfo.TokenName.StartsWith("IEEE64")

let tokenizeFile (inputFileName: string) (outputFileName: string) =
    // Read the input file content
    let sourceText = File.ReadAllText(inputFileName)

    // Create a SourceText from the file content
    let sourceText = SourceText.ofString sourceText

    // Create a tokenizer
    let defines = [] // Add any preprocessor defines if needed
    let tokenizer = FSharpSourceTokenizer(defines, Some inputFileName, None, None)
    let tokenizerState = FSharpTokenizerLexState.Initial
    let lines = sourceText.ToString().Split(Environment.NewLine.ToCharArray(), StringSplitOptions.RemoveEmptyEntries)

    // Tokenize each line and accumulate the tokens
    let lines =
        lines
        |> Seq.mapi (fun lineNumber line ->
            let lineTokenizer = tokenizer.CreateLineTokenizer(line)
            let rec tokenizeLine state tokens =
                match lineTokenizer.ScanToken(state) with
                | Some (tokenInfo: FSharpTokenInfo), newState ->
                    let tokenText = line.Substring(tokenInfo.LeftColumn, tokenInfo.RightColumn - tokenInfo.LeftColumn + 1)
                    let modifiedTokenText =
                        if isFloatLiteral(tokenInfo) then
                            wrapFloatToken(tokenText)
                        else
                            tokenText
                    tokenizeLine newState (modifiedTokenText :: tokens)
                | None, _ -> List.rev tokens
            let tokens = tokenizeLine FSharpTokenizerLexState.Initial  []
            String.concat "" tokens)

    // Write the tokens to the output file
    let output = String.concat "\n" lines
    File.WriteAllText(outputFileName, output)

[<EntryPoint>]
let main argv =
    if argv.Length <> 2 then
        printfn "Usage: Tokenize <InputFSharpFile> <OutputFile>"
        1
    else
        let inputFileName = argv.[0]
        let outputFileName = argv.[1]
        tokenizeFile inputFileName outputFileName
        printfn "Tokenization complete. Output written to: %s" outputFileName
        0
