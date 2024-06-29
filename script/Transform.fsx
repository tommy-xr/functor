#r "nuget: FSharp.Compiler.Service, 43.8.300"

open System
open System.IO
open FSharp.Compiler.Text
open FSharp.Compiler.Tokenization

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
    let tokens =
        lines
        |> Array.mapi (fun i line ->
            let lineNumber = i + 1
            let lineTokenizer = tokenizer.CreateLineTokenizer(line)
            let rec tokenizeLine (state, tokens) =
                match lineTokenizer.ScanToken(state) with
                | Some tokenInfo, newState ->
                    tokenizeLine (newState, tokenInfo :: tokens)
                | None, _ ->
                    List.rev tokens
            tokenizeLine (tokenizerState, []))
        |> Array.map List.toArray
        |> Array.collect id

    // Write the tokens to the output file
    let output = tokens |> Array.map (fun token -> token.TokenName) |> String.concat " "
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
